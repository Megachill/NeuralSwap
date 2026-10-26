//! UI languages. The English text is the key: `t("Install DLSS 5")` returns
//! the German or Spanish version, or the English text itself when a string
//! has no translation. `tr!("{} of {} shown", a, b)` does the same for text
//! with values in it (`{}` in order, or `{0}` / `{1}` to reorder).
//!
//! Technical log lines from the installer stay English on purpose: they are
//! diagnostics people paste into bug reports.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En = 0,
    De = 1,
    Es = 2,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::De, Lang::Es];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Es => "es",
        }
    }

    /// Always shown in its own language, so anyone can find theirs.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Es => "Español",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code().eq_ignore_ascii_case(code.trim()))
    }

    /// The Windows display language, when it is one we have.
    pub fn system() -> Lang {
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        match id & 0x3ff {
            0x07 => Lang::De,
            0x0a => Lang::Es,
            _ => Lang::En,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::De,
        2 => Lang::Es,
        _ => Lang::En,
    }
}

/// The saved choice, else the Windows display language.
pub fn init(saved: Option<&str>) {
    set(saved.and_then(Lang::from_code).unwrap_or_else(Lang::system));
}

static TABLE: Lazy<HashMap<&'static str, (&'static str, &'static str)>> =
    Lazy::new(|| crate::i18n_table::ENTRIES.iter().map(|(en, de, es)| (*en, (*de, *es))).collect());

pub fn t(en: &'static str) -> &'static str {
    match current() {
        Lang::En => en,
        lang => TABLE.get(en).map(|(de, es)| if lang == Lang::De { *de } else { *es }).unwrap_or(en),
    }
}

/// Fills `{}` (in order) or `{0}`, `{1}`… into a translated template.
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) if after[..close].chars().all(|c| c.is_ascii_digit()) => {
                let idx = if close == 0 {
                    next += 1;
                    next - 1
                } else {
                    after[..close].parse().unwrap_or(0)
                };
                if let Some(a) = args.get(idx) {
                    out.push_str(&a.to_string());
                }
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[macro_export]
macro_rules! tr {
    ($s:literal) => {
        $crate::i18n::t($s)
    };
    ($s:literal, $($a:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::t($s), &[$(&$a as &dyn std::fmt::Display),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[test]
    fn fill_handles_order_and_indexes() {
        assert_eq!(fill("{} of {} shown", &[&3, &9]), "3 of 9 shown");
        assert_eq!(fill("{1} von {0}", &[&"a", &"b"]), "b von a");
        assert_eq!(fill("keep {braces", &[&1]), "keep {braces");
    }

    fn unescape(s: &str) -> String {
        s.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\")
    }

    /// Every string the UI translates must have German and Spanish versions,
    /// with the same placeholders as the English one.
    #[test]
    fn every_ui_string_is_translated() {
        let re = Regex::new(r#"(?:\bt|tr!)\(\s*"((?:[^"\\]|\\.)*)""#).unwrap();
        let mut keys: Vec<String> = Vec::new();
        for file in ["src/ui/pages.rs", "src/ui/sheet.rs", "src/ui/widgets.rs", "src/ui/installer.rs", "src/ui/mod.rs", "src/components.rs"] {
            let text = std::fs::read_to_string(file).unwrap();
            for c in re.captures_iter(&text) {
                keys.push(unescape(&c[1]));
            }
        }
        // Strings translated through a variable rather than a literal.
        for r in [crate::core::apply::Route::Native, crate::core::apply::Route::Renodx, crate::core::apply::Route::Feeder] {
            keys.push(r.label().to_string());
        }
        for c in crate::components::CORE {
            keys.push(c.label.to_string());
        }
        keys.push("Home".to_string());
        for code in crate::core::ERROR_CODES {
            keys.push(crate::core::fail(code, "").message().to_string());
        }
        keys.sort();
        keys.dedup();
        let placeholders = |s: &str| Regex::new(r"\{\d*\}").unwrap().find_iter(s).count();
        let mut missing = Vec::new();
        for k in &keys {
            match TABLE.get(k.as_str()) {
                None => missing.push(format!("MISSING: {k:?}")),
                Some((de, es)) => {
                    for (lang, v) in [("de", de), ("es", es)] {
                        if v.trim().is_empty() || placeholders(v) != placeholders(k) {
                            missing.push(format!("BAD {lang}: {k:?} -> {v:?}"));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "{} problems:\n{}", missing.len(), missing.join("\n"));
    }
}
