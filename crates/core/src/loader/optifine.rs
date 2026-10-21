//! OptiFine, installed from a jar the user downloaded from optifine.net. Its author forbids
//! automated downloads, so we never fetch it ourselves.
//!
//! Method (as HMCL does it): OptiFine's own `optifine.Patcher` builds the patched client
//! library from the vanilla jar, and a version JSON starts the game through launchwrapper
//! with OptiFine's tweaker.

use std::path::Path;

use serde_json::{Value, json};

use crate::download::{self, Download, ProgressFn};
use crate::error::{Result, other};
use crate::java;
use crate::paths::{Paths, ensure_parent};
use crate::rules::Environment;
use crate::version::{self, JavaVersion, maven_path};

/// `OptiFine_1.20.1_HD_U_I6.jar` -> ("1.20.1", "HD_U_I6"). Also accepts `preview_` names.
pub fn parse_file_name(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".jar")?;
    let rest = stem
        .strip_prefix("preview_")
        .unwrap_or(stem)
        .strip_prefix("OptiFine_")?;
    let (mc, edition) = rest.split_once("_HD_")?;
    Some((mc.to_string(), format!("HD_{edition}")))
}

pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    jar: &Path,
    mc: &str,
    progress: &ProgressFn,
) -> Result<String> {
    let file_name = jar.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let (jar_mc, edition) = parse_file_name(file_name).ok_or_else(|| {
        other("this doesn't look like an OptiFine jar (expected a name like OptiFine_1.20.1_HD_U_I6.jar)")
    })?;
    if jar_mc != mc {
        return Err(other(format!(
            "this OptiFine is for Minecraft {jar_mc}, but the instance is {mc}"
        )));
    }
    let id = format!("{mc}-OptiFine_{edition}");
    let library_version = format!("{mc}_{edition}");

    // Vanilla jar to patch, and the Java it needs.
    let vanilla = version::resolve(http, paths, mc).await?;
    if let Some(client) = vanilla.downloads.as_ref().and_then(|d| d.client.clone()) {
        let dl = Download {
            url: client.url,
            dest: paths.version_jar(mc),
            sha1: client.sha1,
            size: client.size,
        };
        download::fetch_all(http, "Minecraft", vec![dl], progress).await?;
    }
    let required = vanilla.java_version.clone().unwrap_or(JavaVersion {
        component: Some("jre-legacy".into()),
        major_version: 8,
    });
    let java = java::resolve(http, paths, &Environment::current(), &required, progress)
        .await?
        .java;

    let patched = paths
        .libraries()
        .join(maven_path(&format!("optifine:OptiFine:{library_version}"), None).unwrap());
    ensure_parent(&patched)?;
    let out = tokio::process::Command::new(&java)
        .arg("-cp")
        .arg(jar)
        .arg("optifine.Patcher")
        .arg(paths.version_jar(mc))
        .arg(jar)
        .arg(&patched)
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() || !patched.exists() {
        let log = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        return Err(other(format!("OptiFine's patcher failed:\n{}", log.trim())));
    }

    // Newer OptiFine ships its own launchwrapper fork, which works on modern Java.
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
    let wrapper = match zip.by_name("launchwrapper-of.txt") {
        Ok(mut f) => {
            let mut v = String::new();
            std::io::Read::read_to_string(&mut f, &mut v)?;
            Some(v.trim().to_string())
        }
        Err(_) => None,
    };
    let wrapper_library = match wrapper {
        Some(v) => {
            let dest = paths
                .libraries()
                .join(maven_path(&format!("optifine:launchwrapper-of:{v}"), None).unwrap());
            if !dest.exists() {
                let mut entry = zip.by_name(&format!("launchwrapper-of-{v}.jar"))?;
                ensure_parent(&dest)?;
                std::io::copy(&mut entry, &mut std::fs::File::create(&dest)?)?;
            }
            json!({ "name": format!("optifine:launchwrapper-of:{v}"), "downloads": { "artifact": { "url": "" } } })
        }
        None => json!({ "name": "net.minecraft:launchwrapper:1.12" }),
    };

    let mut profile = json!({
        "id": id,
        "inheritsFrom": mc,
        "type": "release",
        "mainClass": "net.minecraft.launchwrapper.Launch",
        "libraries": [
            { "name": format!("optifine:OptiFine:{library_version}"), "downloads": { "artifact": { "url": "" } } },
            wrapper_library,
        ],
    });
    match &vanilla.minecraft_arguments {
        Some(legacy) => {
            profile["minecraftArguments"] =
                Value::String(format!("{legacy} --tweakClass optifine.OptiFineTweaker"))
        }
        None => {
            profile["arguments"] = json!({ "game": ["--tweakClass", "optifine.OptiFineTweaker"] })
        }
    }
    let file = paths.version_json(&id);
    ensure_parent(&file)?;
    std::fs::write(&file, serde_json::to_vec_pretty(&profile)?)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::parse_file_name;

    #[test]
    fn parses_optifine_names() {
        assert_eq!(
            parse_file_name("OptiFine_1.20.1_HD_U_I6.jar"),
            Some(("1.20.1".into(), "HD_U_I6".into()))
        );
        assert_eq!(
            parse_file_name("preview_OptiFine_1.21.1_HD_U_J1_pre9.jar"),
            Some(("1.21.1".into(), "HD_U_J1_pre9".into()))
        );
        assert_eq!(parse_file_name("sodium.jar"), None);
    }
}
