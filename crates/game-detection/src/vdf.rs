use std::collections::BTreeMap;

use crate::DetectionError;

/// Parsed Valve KeyValues (VDF/ACF) node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfValue {
    String(String),
    Object(BTreeMap<String, VdfValue>),
}

impl VdfValue {
    pub fn as_object(&self) -> Option<&BTreeMap<String, VdfValue>> {
        match self {
            Self::Object(map) => Some(map),
            Self::String(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            Self::Object(_) => None,
        }
    }

    /// Case-insensitive field lookup on an object node.
    pub fn get_ci(&self, key: &str) -> Option<&VdfValue> {
        let obj = self.as_object()?;
        obj.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_str_ci(&self, key: &str) -> Option<&str> {
        self.get_ci(key).and_then(VdfValue::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Atom(String),
    LBrace,
    RBrace,
}

fn tokenize(input: &str) -> Result<Vec<Token>, DetectionError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        match ch {
            '{' => {
                tokens.push(Token::LBrace);
                i += 1;
            }
            '}' => {
                tokens.push(Token::RBrace);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut buf = String::new();
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '"' {
                        i += 1;
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        i += 1;
                        if i >= chars.len() {
                            return Err(DetectionError::MetadataParse(
                                "unterminated escape in VDF string".to_owned(),
                            ));
                        }
                        let escaped = match chars[i] {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '\\' => '\\',
                            '"' => '"',
                            other => other,
                        };
                        buf.push(escaped);
                        i += 1;
                    } else {
                        buf.push(c);
                        i += 1;
                    }
                }
                if !closed {
                    return Err(DetectionError::MetadataParse(
                        "unterminated quoted string in VDF".to_owned(),
                    ));
                }
                tokens.push(Token::Atom(buf));
            }
            _ => {
                let mut buf = String::new();
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != '{'
                    && chars[i] != '}'
                    && chars[i] != '"'
                {
                    buf.push(chars[i]);
                    i += 1;
                }
                tokens.push(Token::Atom(buf));
            }
        }
    }

    Ok(tokens)
}

fn parse_object(
    tokens: &[Token],
    pos: &mut usize,
    expect_rbrace: bool,
) -> Result<VdfValue, DetectionError> {
    let mut map = BTreeMap::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::RBrace => {
                if expect_rbrace {
                    *pos += 1;
                    return Ok(VdfValue::Object(map));
                }
                return Err(DetectionError::MetadataParse(
                    "unexpected '}' at top level of VDF".to_owned(),
                ));
            }
            Token::Atom(key) => {
                *pos += 1;
                if *pos >= tokens.len() {
                    return Err(DetectionError::MetadataParse(format!(
                        "missing value for VDF key '{key}'"
                    )));
                }
                let val = match &tokens[*pos] {
                    Token::Atom(s) => {
                        *pos += 1;
                        VdfValue::String(s.clone())
                    }
                    Token::LBrace => {
                        *pos += 1;
                        parse_object(tokens, pos, true)?
                    }
                    Token::RBrace => {
                        return Err(DetectionError::MetadataParse(format!(
                            "unexpected '}}' after VDF key '{key}'"
                        )));
                    }
                };
                map.insert(key.clone(), val);
            }
            Token::LBrace => {
                return Err(DetectionError::MetadataParse(
                    "unexpected '{' where VDF key was expected".to_owned(),
                ));
            }
        }
    }

    if expect_rbrace {
        return Err(DetectionError::MetadataParse(
            "unclosed '{' block in VDF".to_owned(),
        ));
    }

    Ok(VdfValue::Object(map))
}

/// Parses a Valve KeyValues (`*.vdf` / `*.acf`) text document.
pub fn parse_vdf(input: &str) -> Result<VdfValue, DetectionError> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    parse_object(&tokens, &mut pos, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_vdf_with_escaped_windows_paths() {
        let sample = r#"
        // Steam library folders
        "libraryfolders"
        {
            "0"
            {
                "path"      "C:\\Program Files (x86)\\Steam"
                "label"     ""
                "apps"
                {
                    "367520"    "123456"
                }
            }
        }
        "#;

        let root = parse_vdf(sample).expect("valid VDF");
        let lf = root.get_ci("libraryfolders").expect("libraryfolders root");
        let zero = lf.get_ci("0").expect("entry 0");
        assert_eq!(
            zero.get_str_ci("path"),
            Some("C:\\Program Files (x86)\\Steam")
        );
    }

    #[test]
    fn rejects_unclosed_brace() {
        let bad = r#""AppState" { "appid" "123" "#;
        assert!(matches!(
            parse_vdf(bad),
            Err(DetectionError::MetadataParse(_))
        ));
    }
}
