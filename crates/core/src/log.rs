//! Turns the game's log4j XML output (from Mojang's log config) back into plain log lines.
//!
//! Mojang's config is kept on purpose: for 1.7-1.18 it also carries the Log4Shell mitigation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: String,
    pub thread: String,
    pub text: String,
}

/// Feed stdout lines in order; returns a finished line whenever one is complete.
#[derive(Debug, Default)]
pub struct XmlLogParser {
    level: String,
    thread: String,
    /// Collecting a multi-line CDATA block (stack traces, multi-line messages).
    pending: Option<String>,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

impl XmlLogParser {
    pub fn feed(&mut self, line: &str) -> Option<LogLine> {
        if let Some(buf) = &mut self.pending {
            match line.find("]]>") {
                Some(end) => {
                    buf.push('\n');
                    buf.push_str(&line[..end]);
                    let text = self.pending.take().unwrap();
                    return Some(self.emit(text));
                }
                None => {
                    buf.push('\n');
                    buf.push_str(line);
                    return None;
                }
            }
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with("<log4j:Event") {
            self.level = attr(trimmed, "level").unwrap_or_default();
            self.thread = attr(trimmed, "thread").unwrap_or_default();
            return None;
        }
        if let Some(start) = trimmed.find("<![CDATA[") {
            let body = &trimmed[start + 9..];
            return match body.find("]]>") {
                Some(end) => Some(self.emit(body[..end].to_string())),
                None => {
                    self.pending = Some(body.to_string());
                    None
                }
            };
        }
        if trimmed.starts_with("</log4j:") || trimmed.starts_with("<log4j:") || trimmed.is_empty() {
            return None;
        }
        // Not XML: early JVM output, or a version without the XML config.
        Some(LogLine {
            level: String::new(),
            thread: String::new(),
            text: line.to_string(),
        })
    }

    fn emit(&self, text: String) -> LogLine {
        LogLine {
            level: self.level.clone(),
            thread: self.thread.clone(),
            text,
        }
    }
}

impl std::fmt::Display for LogLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.level.is_empty() {
            write!(f, "{}", self.text)
        } else {
            write!(f, "[{}/{}] {}", self.thread, self.level, self.text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_and_multiline_throwables() {
        let input = [
            r#"<log4j:Event logger="x" timestamp="1" level="INFO" thread="Render thread">"#,
            "  <log4j:Message><![CDATA[Setting user: Steve]]></log4j:Message>",
            "</log4j:Event>",
            r#"<log4j:Event logger="x" timestamp="2" level="ERROR" thread="Download-1">"#,
            "  <log4j:Throwable><![CDATA[java.lang.Exception: boom",
            "\tat a.b(C.java:1)",
            "]]></log4j:Throwable>",
            "plain jvm output",
        ];
        let mut p = XmlLogParser::default();
        let out: Vec<String> = input
            .iter()
            .filter_map(|l| p.feed(l))
            .map(|l| l.to_string())
            .collect();
        assert_eq!(
            out,
            [
                "[Render thread/INFO] Setting user: Steve",
                "[Download-1/ERROR] java.lang.Exception: boom\n\tat a.b(C.java:1)\n",
                "plain jvm output",
            ]
        );
    }
}
