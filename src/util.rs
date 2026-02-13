use chrono::{Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Since {
    None,
    All,
    Latest,
    Time(i64),
}

impl Since {
    pub fn parse(s: &str) -> Self {
        if s == "all" {
            return Since::All;
        }
        if s == "none" {
            return Since::None;
        }
        if s == "latest" {
            return Since::Latest;
        }
        if let Ok(ts) = parse_future_time(s) {
             return Since::Time(ts);
        }
        Since::None
    }
}

use crate::types::Action;
use std::collections::HashMap;

pub fn parse_actions(s: &str) -> Result<Vec<Action>, String> {
    let s = s.trim();
    if s.starts_with('[') {
        return serde_json::from_str(s).map_err(|e| format!("JSON error: {}", e));
    }
    
    let mut parser = ActionParser::new(s);
    parser.parse()
}

struct ActionParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> ActionParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(&mut self) -> Result<Vec<Action>, String> {
        let mut actions = Vec::new();
        while !self.eof() {
            let a = self.parse_action()?;
            actions.push(a);
            self.slurp_spaces();
        }
        Ok(actions)
    }

    fn parse_action(&mut self) -> Result<Action, String> {
        let mut a = Action {
            id: rand::random::<u64>().to_string().chars().take(10).collect(), // Simplified ID
            action: String::new(),
            label: String::new(),
            clear: false,
            url: None,
            method: None,
            headers: None,
            body: None,
            intent: None,
            extras: None,
            value: None,
        };
        
        let mut section = 0;
        loop {
            let (key, value, last) = self.parse_section()?;
            self.populate_action(&mut a, section, key, value)?;
            self.slurp_spaces();
            if last {
                return Ok(a);
            }
            section += 1;
        }
    }

    fn populate_action(&self, a: &mut Action, section: usize, key: String, value: String) -> Result<(), String> {
        let mut key = key;
        if key.is_empty() {
            match section {
                0 => key = "action".to_string(),
                1 => key = "label".to_string(),
                2 => {
                    if a.action == "view" || a.action == "http" {
                        key = "url".to_string();
                    } else if a.action == "copy" {
                        key = "value".to_string();
                    }
                }
                _ => {}
            }
        }

        if key.is_empty() {
            return Err(format!("Unknown term '{}'", value));
        }

        if key.starts_with("headers.") {
            let header_key = &key[8..];
            let headers = a.headers.get_or_insert_with(HashMap::new);
            headers.insert(header_key.to_string(), value);
        } else if key.starts_with("extras.") {
            let extra_key = &key[7..];
            let extras = a.extras.get_or_insert_with(HashMap::new);
            extras.insert(extra_key.to_string(), value);
        } else {
            match key.to_lowercase().as_str() {
                "action" => a.action = value,
                "label" => a.label = value,
                "clear" => {
                    let lval = value.to_lowercase();
                    a.clear = lval == "true" || lval == "yes" || lval == "1";
                }
                "url" => a.url = Some(value),
                "method" => a.method = Some(value.to_uppercase()),
                "body" => a.body = Some(value),
                "value" => a.value = Some(value),
                "intent" => a.intent = Some(value),
                _ => return Err(format!("Unknown key '{}'", key)),
            }
        }
        Ok(())
    }

    fn parse_section(&mut self) -> Result<(String, String, bool), String> {
        self.slurp_spaces();
        let key = self.parse_key();
        
        let (r, _) = self.peek();
        if self.is_section_end(r) {
            self.pos += r.len_utf8();
            return Ok((key, String::new(), self.is_last_section(r)));
        } else if r == '"' || r == '\'' {
            let (val, last) = self.parse_quoted_value(r)?;
            return Ok((key, val, last));
        }
        
        let (val, last) = self.parse_value();
        Ok((key, val, last))
    }

    fn parse_key(&mut self) -> String {
        let re = regex::Regex::new(r"^([-.\w]+)\s*=\s*").unwrap();
        if let Some(caps) = re.captures(&self.input[self.pos..]) {
            let full_match = caps.get(0).unwrap();
            self.pos += full_match.end();
            return caps.get(1).unwrap().as_str().to_string();
        }
        String::new()
    }

    fn parse_value(&mut self) -> (String, bool) {
        let start = self.pos;
        loop {
            let (r, w) = self.peek();
            if self.is_section_end(r) {
                let last = self.is_last_section(r);
                let val = self.input[start..self.pos].trim().to_string();
                self.pos += w;
                return (val, last);
            }
            self.pos += w;
        }
    }

    fn parse_quoted_value(&mut self, quote: char) -> Result<(String, bool), String> {
        self.pos += quote.len_utf8();
        let start = self.pos;
        let mut prev = '\0';
        loop {
            let (r, w) = self.peek();
            if r == '\0' {
                return Err("Unexpected EOF in quoted string".to_string());
            }
            if r == quote && prev != '\\' {
                let val = self.input[start..self.pos].replace(&format!("\\{}", quote), &quote.to_string());
                self.pos += w;
                self.slurp_spaces();
                let (r2, w2) = self.peek();
                let last = self.is_last_section(r2);
                if !self.is_section_end(r2) {
                    return Err(format!("Unexpected character '{}' after quoted string", r2));
                }
                self.pos += w2;
                return Ok((val, last));
            }
            prev = r;
            self.pos += w;
        }
    }

    fn slurp_spaces(&mut self) {
        while !self.eof() {
            let (r, w) = self.peek();
            if r.is_whitespace() {
                self.pos += w;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> (char, usize) {
        if self.eof() {
            return ('\0', 0);
        }
        let c = self.input[self.pos..].chars().next().unwrap();
        (c, c.len_utf8())
    }

    fn eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn is_section_end(&self, c: char) -> bool {
        c == '\0' || c == ';' || c == ','
    }

    fn is_last_section(&self, c: char) -> bool {
        c == '\0' || c == ';'
    }
}

pub fn parse_future_time(s: &str) -> Result<i64, String> {
    if let Ok(ts) = s.parse::<i64>() {
        return Ok(ts);
    }
    
    let re = regex::Regex::new(r"^(\d+)([smhd])$").unwrap();
    if let Some(caps) = re.captures(s) {
        let amount: i64 = caps[1].parse().map_err(|_| "Invalid amount")?;
        let unit = &caps[2];
        
        let duration = match unit {
            "s" => Duration::seconds(amount),
            "m" => Duration::minutes(amount),
            "h" => Duration::hours(amount),
            "d" => Duration::days(amount),
            _ => return Err("Invalid unit".to_string()),
        };
        
        return Ok((Utc::now() + duration).timestamp());
    }
    
    Err("Invalid time format".to_string())
}
