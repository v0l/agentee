#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    List(Vec<Node>),
    Atom(String),
    Str(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected end of input")]
    Eof,
    #[error("unexpected `)` at byte {0}")]
    Close(usize),
    #[error("unterminated string at byte {0}")]
    String(usize),
    #[error("trailing data at byte {0}")]
    Trailing(usize),
}

pub fn parse(src: &str) -> Result<Node, ParseError> {
    let b = src.as_bytes();
    let mut i = 0;
    let n = node(src, b, &mut i)?;
    skip_ws(b, &mut i);
    if i < b.len() {
        return Err(ParseError::Trailing(i));
    }
    Ok(n)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn node(src: &str, b: &[u8], i: &mut usize) -> Result<Node, ParseError> {
    skip_ws(b, i);
    match b.get(*i) {
        None => Err(ParseError::Eof),
        Some(b'(') => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(b, i);
                match b.get(*i) {
                    None => return Err(ParseError::Eof),
                    Some(b')') => {
                        *i += 1;
                        return Ok(Node::List(items));
                    }
                    _ => items.push(node(src, b, i)?),
                }
            }
        }
        Some(b')') => Err(ParseError::Close(*i)),
        Some(b'"') => {
            let start = *i;
            *i += 1;
            let mut s = String::new();
            loop {
                match b.get(*i) {
                    None => return Err(ParseError::String(start)),
                    Some(b'"') => {
                        *i += 1;
                        return Ok(Node::Str(s));
                    }
                    Some(b'\\') => {
                        match b.get(*i + 1) {
                            Some(b'n') => s.push('\n'),
                            Some(b't') => s.push('\t'),
                            Some(&c) => s.push(c as char),
                            None => return Err(ParseError::String(start)),
                        }
                        *i += 2;
                    }
                    Some(_) => {
                        let rest = &src[*i..];
                        let c = rest.chars().next().unwrap();
                        s.push(c);
                        *i += c.len_utf8();
                    }
                }
            }
        }
        Some(_) => {
            let start = *i;
            while *i < b.len() && !b[*i].is_ascii_whitespace() && b[*i] != b'(' && b[*i] != b')' {
                *i += 1;
            }
            Ok(Node::Atom(src[start..*i].to_string()))
        }
    }
}

impl Node {
    pub fn items(&self) -> &[Node] {
        match self {
            Node::List(v) => v,
            _ => &[],
        }
    }

    pub fn head(&self) -> Option<&str> {
        match self.items().first() {
            Some(Node::Atom(s)) => Some(s),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Node::Atom(s) | Node::Str(s) => Some(s),
            Node::List(_) => None,
        }
    }

    pub fn arg(&self, i: usize) -> Option<&str> {
        self.items().get(i + 1).and_then(Node::text)
    }

    pub fn num(&self, i: usize) -> Option<f64> {
        self.arg(i)?.parse().ok()
    }

    pub fn find(&self, name: &str) -> Option<&Node> {
        self.items().iter().find(|n| n.head() == Some(name))
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items().iter().filter(move |n| n.head() == Some(name))
    }

    pub fn has_atom(&self, atom: &str) -> bool {
        self.items().iter().skip(1).any(|n| matches!(n, Node::Atom(a) if a == atom))
    }

    pub fn xy(&self, name: &str) -> Option<[f64; 2]> {
        let n = self.find(name)?;
        Some([n.num(0)?, n.num(1)?])
    }

    pub fn flag(&self, name: &str) -> bool {
        if self.has_atom(name) {
            return true;
        }
        match self.find(name) {
            Some(n) => n.arg(0).map(|v| v == "yes").unwrap_or(true),
            None => false,
        }
    }

    pub fn pts(&self) -> Vec<[f64; 2]> {
        self.find("pts")
            .map(|p| p.all("xy").filter_map(|xy| Some([xy.num(0)?, xy.num(1)?])).collect())
            .unwrap_or_default()
    }

    pub fn property(&self, key: &str) -> Option<&Node> {
        self.all("property").find(|p| p.arg(0) == Some(key))
    }

    pub fn property_text(&self, key: &str) -> Option<String> {
        self.property(key).and_then(|p| p.arg(1)).map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_lists_and_strings() {
        let n = parse(r#"(a (b 1 2) "c \"d\"" (e))"#).unwrap();
        assert_eq!(n.head(), Some("a"));
        assert_eq!(n.find("b").unwrap().num(1), Some(2.0));
        assert_eq!(n.arg(1), Some("c \"d\""));
    }
}
