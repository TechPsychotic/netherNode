//! Evaluation of the `rules` arrays found on libraries and arguments in version JSONs.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: Action,
    pub os: Option<OsRule>,
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    /// Regex matched against the OS version string.
    pub version: Option<String>,
    #[serde(rename = "versionRange")]
    pub version_range: Option<VersionRange>,
}

/// `min` is inclusive, `max` exclusive (Mojang uses the same bound for both sides of a split).
#[derive(Debug, Clone, Deserialize)]
pub struct VersionRange {
    pub min: Option<String>,
    pub max: Option<String>,
}

/// Facts about the machine and the launch that rules are checked against.
#[derive(Debug, Clone)]
pub struct Environment {
    /// Mojang naming: `windows`, `osx` or `linux`.
    pub os_name: &'static str,
    /// Mojang naming: `x86`, `x86_64` or `arm64`.
    pub arch: &'static str,
    pub os_version: String,
    /// Enabled features, e.g. `has_custom_resolution`. Anything absent counts as `false`.
    pub features: HashSet<String>,
}

impl Environment {
    pub fn current() -> Self {
        let os_name = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            _ => "linux",
        };
        let arch = match std::env::consts::ARCH {
            "x86" => "x86",
            "aarch64" => "arm64",
            _ => "x86_64",
        };
        Self {
            os_name,
            arch,
            os_version: os_info::get().version().to_string(),
            features: HashSet::new(),
        }
    }

    /// `"64"` or `"32"`, substituted for `${arch}` in legacy native classifiers.
    pub fn arch_bits(&self) -> &'static str {
        if self.arch == "x86" { "32" } else { "64" }
    }
}

/// No rules means allowed; otherwise start disallowed and let the last matching rule decide.
pub fn allowed(rules: &[Rule], env: &Environment) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut result = Action::Disallow;
    for rule in rules {
        if rule_matches(rule, env) {
            result = rule.action;
        }
    }
    result == Action::Allow
}

fn rule_matches(rule: &Rule, env: &Environment) -> bool {
    if let Some(os) = &rule.os {
        if os.name.as_deref().is_some_and(|n| n != env.os_name) {
            return false;
        }
        if os.arch.as_deref().is_some_and(|a| a != env.arch) {
            return false;
        }
        if let Some(pattern) = &os.version {
            match regex::Regex::new(pattern) {
                Ok(re) if re.is_match(&env.os_version) => {}
                _ => return false,
            }
        }
        if let Some(range) = &os.version_range
            && !in_range(&env.os_version, range)
        {
            return false;
        }
    }
    if let Some(features) = &rule.features {
        for (name, wanted) in features {
            if env.features.contains(name) != *wanted {
                return false;
            }
        }
    }
    true
}

fn parse_version(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn in_range(version: &str, range: &VersionRange) -> bool {
    let v = parse_version(version);
    if v.is_empty() {
        return false;
    }
    if let Some(min) = &range.min
        && v < parse_version(min)
    {
        return false;
    }
    if let Some(max) = &range.max
        && v >= parse_version(max)
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: &'static str, arch: &'static str, version: &str) -> Environment {
        Environment {
            os_name: os,
            arch,
            os_version: version.into(),
            features: HashSet::new(),
        }
    }

    fn rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_rules_allow() {
        assert!(allowed(&[], &env("linux", "x86_64", "6.0")));
    }

    #[test]
    fn allow_all_but_osx() {
        let r = rules(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#);
        assert!(allowed(&r, &env("linux", "x86_64", "")));
        assert!(!allowed(&r, &env("osx", "arm64", "")));
    }

    #[test]
    fn arch_rule() {
        let r = rules(r#"[{"action":"allow","os":{"arch":"x86"}}]"#);
        assert!(allowed(&r, &env("windows", "x86", "")));
        assert!(!allowed(&r, &env("windows", "x86_64", "")));
    }

    #[test]
    fn version_range_split() {
        let new = rules(
            r#"[{"action":"allow","os":{"name":"windows","versionRange":{"min":"10.0.17134"}}}]"#,
        );
        let old = rules(
            r#"[{"action":"allow","os":{"name":"windows","versionRange":{"max":"10.0.17134"}}}]"#,
        );
        let win11 = env("windows", "x86_64", "10.0.22631");
        let win_old = env("windows", "x86_64", "10.0.10240");
        assert!(allowed(&new, &win11) && !allowed(&old, &win11));
        assert!(!allowed(&new, &win_old) && allowed(&old, &win_old));
    }

    #[test]
    fn features() {
        let r = rules(r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#);
        let mut e = env("linux", "x86_64", "");
        assert!(!allowed(&r, &e));
        e.features.insert("has_custom_resolution".into());
        assert!(allowed(&r, &e));
    }
}
