//! Match rules (AddMatch): which broadcast signals a connection receives.
//!
//! `type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='x'`
//! Values are single-quoted; outside quotes `\'` is a literal quote.
//! `eavesdrop='true'` is accepted and ignored: there is no eavesdropping
//! (whitepaper §6.8).

use crate::message::{ERROR, METHOD_CALL, METHOD_RETURN, Message, SIGNAL};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MatchRule {
    /// The rule as the client wrote it (RemoveMatch names rules by it).
    pub text: String,
    pub kind: Option<u8>,
    pub sender: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub path: Option<String>,
    pub path_namespace: Option<String>,
    pub destination: Option<String>,
    /// argN='value' (exact string) and argNpath='value' (path prefix rules).
    pub args: Vec<(usize, String, bool)>,
    pub arg0namespace: Option<String>,
}

fn split_pairs(text: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut key = String::new();
        loop {
            match chars.next()? {
                '=' => break,
                c => key.push(c),
            }
        }
        let mut val = String::new();
        let mut quoted = false;
        loop {
            match chars.next() {
                None => break,
                Some('\'') => quoted = !quoted,
                Some('\\') if !quoted && chars.peek() == Some(&'\'') => {
                    chars.next();
                    val.push('\'');
                }
                Some(',') if !quoted => break,
                Some(c) => val.push(c),
            }
        }
        if quoted {
            return None;
        }
        out.push((key.trim().to_string(), val));
    }
    Some(out)
}

impl MatchRule {
    pub fn parse(text: &str) -> Option<MatchRule> {
        let mut r = MatchRule { text: text.into(), ..Default::default() };
        for (k, v) in split_pairs(text)? {
            match k.as_str() {
                "type" => {
                    r.kind = Some(match v.as_str() {
                        "signal" => SIGNAL,
                        "method_call" => METHOD_CALL,
                        "method_return" => METHOD_RETURN,
                        "error" => ERROR,
                        _ => return None,
                    })
                }
                "sender" => r.sender = Some(v),
                "interface" => r.interface = Some(v),
                "member" => r.member = Some(v),
                "path" => r.path = Some(v),
                "path_namespace" => r.path_namespace = Some(v),
                "destination" => r.destination = Some(v),
                "arg0namespace" => r.arg0namespace = Some(v),
                "eavesdrop" => {}
                k if k.starts_with("arg") => {
                    let rest = &k[3..];
                    let (num, path) = match rest.strip_suffix("path") {
                        Some(n) => (n, true),
                        None => (rest, false),
                    };
                    let n: usize = num.parse().ok().filter(|n| *n < 64)?;
                    r.args.push((n, v, path));
                }
                _ => return None,
            }
        }
        if r.path.is_some() && r.path_namespace.is_some() {
            return None;
        }
        Some(r)
    }

    /// Does `m` match? `sender_names` are every name the sending connection
    /// owns (its unique name first), so a rule on a well-known sender name
    /// matches messages from its owner.
    pub fn matches(&self, m: &Message, sender_names: &[String]) -> bool {
        if self.kind.is_some_and(|k| k != m.kind) {
            return false;
        }
        if let Some(s) = &self.sender
            && !sender_names.iter().any(|n| n == s)
        {
            return false;
        }
        let eq =
            |rule: &Option<String>, field: &Option<String>| rule.as_ref().is_none_or(|r| field.as_ref() == Some(r));
        if !eq(&self.interface, &m.interface)
            || !eq(&self.member, &m.member)
            || !eq(&self.path, &m.path)
            || !eq(&self.destination, &m.destination)
        {
            return false;
        }
        if let Some(ns) = &self.path_namespace {
            let Some(p) = &m.path else { return false };
            let inside = ns == "/" || p == ns || p.strip_prefix(ns.as_str()).is_some_and(|r| r.starts_with('/'));
            if !inside {
                return false;
            }
        }
        if self.args.is_empty() && self.arg0namespace.is_none() {
            return true;
        }
        let Ok(args) = m.args() else { return false };
        let arg_str = |i: usize| args.get(i).and_then(Value::as_str);
        for (i, want, path) in &self.args {
            let Some(got) = arg_str(*i) else { return false };
            let ok = if *path {
                got == want
                    || (want.ends_with('/') && got.starts_with(want.as_str()))
                    || (got.ends_with('/') && want.starts_with(got))
            } else {
                got == want
            };
            if !ok {
                return false;
            }
        }
        if let Some(ns) = &self.arg0namespace {
            let Some(got) = arg_str(0) else { return false };
            if !(got == ns || got.strip_prefix(ns.as_str()).is_some_and(|r| r.starts_with('.'))) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noc(arg0: &str) -> Message {
        Message::signal(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            &[Value::Str(arg0.into()), Value::Str(String::new()), Value::Str(":1.5".into())],
        )
    }

    #[test]
    fn parse_and_match() {
        let r = MatchRule::parse(
            "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.mozilla.firefox'",
        )
        .unwrap();
        let bus = ["org.freedesktop.DBus".to_string()];
        assert!(r.matches(&noc("org.mozilla.firefox"), &bus));
        assert!(!r.matches(&noc("org.other"), &bus));
        assert!(!r.matches(&noc("org.mozilla.firefox"), &[":1.3".to_string()]));
    }

    #[test]
    fn namespaces_and_paths() {
        let r = MatchRule::parse("arg0namespace='org.mozilla'").unwrap();
        let bus = ["org.freedesktop.DBus".to_string()];
        assert!(r.matches(&noc("org.mozilla.firefox"), &bus));
        assert!(r.matches(&noc("org.mozilla"), &bus));
        assert!(!r.matches(&noc("org.mozillafoo"), &bus));
        let r = MatchRule::parse("path_namespace='/org/freedesktop'").unwrap();
        assert!(r.matches(&noc("x"), &bus));
        let r = MatchRule::parse("path_namespace='/org/free'").unwrap();
        assert!(!r.matches(&noc("x"), &bus));
    }

    #[test]
    fn quoting_and_bad_rules() {
        // Inside quotes a backslash is literal; a quote is written by closing,
        // escaping and reopening.
        let r = MatchRule::parse("member='a,b',arg1='it'\\''s'").unwrap();
        assert_eq!(r.member.as_deref(), Some("a,b"));
        assert_eq!(r.args, [(1, "it's".to_string(), false)]);
        let r = MatchRule::parse("arg1=it\\'s").unwrap();
        assert_eq!(r.args, [(1, "it's".to_string(), false)]);
        assert!(MatchRule::parse("type='bogus'").is_none());
        assert!(MatchRule::parse("frobnicate='x'").is_none());
        assert!(MatchRule::parse("member='unterminated").is_none());
        assert!(MatchRule::parse("path='/a',path_namespace='/a'").is_none());
        assert!(MatchRule::parse("eavesdrop='true',type='signal'").is_some());
        assert!(MatchRule::parse("").is_some());
    }
}
