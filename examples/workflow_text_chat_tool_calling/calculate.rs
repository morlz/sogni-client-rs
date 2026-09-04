use serde_json::json;

pub fn evaluate(expression: &str) -> String {
    if expression.len() > 512 {
        return json!({"error": "expression exceeds 512 characters"}).to_string();
    }
    let result = tokenize(expression).and_then(|tokens| Parser::new(tokens).parse());
    match result {
        Ok(value) if value.is_finite() => {
            json!({"expression": expression, "result": value}).to_string()
        }
        Ok(value) => {
            json!({"error": format!("expression produced non-finite value {value}")}).to_string()
        }
        Err(error) => json!({"error": format!("invalid expression: {error}")}).to_string(),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Open,
    Close,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            value if value.is_whitespace() => index += 1,
            value if value.is_ascii_digit() || value == '.' => {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_digit()
                        || matches!(chars[index], '.' | 'e' | 'E')
                        || matches!(chars[index], '+' | '-')
                            && matches!(chars[index - 1], 'e' | 'E'))
                {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                tokens.push(Token::Number(
                    text.parse().map_err(|_| format!("bad number {text:?}"))?,
                ));
            }
            value if value.is_ascii_alphabetic() || value == '_' => {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                tokens.push(Token::Name(chars[start..index].iter().collect()));
            }
            '+' => push(&mut tokens, Token::Add, &mut index),
            '-' => push(&mut tokens, Token::Sub, &mut index),
            '/' => push(&mut tokens, Token::Div, &mut index),
            '%' => push(&mut tokens, Token::Mod, &mut index),
            '^' => push(&mut tokens, Token::Pow, &mut index),
            '(' => push(&mut tokens, Token::Open, &mut index),
            ')' => push(&mut tokens, Token::Close, &mut index),
            ',' => push(&mut tokens, Token::Comma, &mut index),
            '*' => {
                index += 1;
                if chars.get(index) == Some(&'*') {
                    index += 1;
                    tokens.push(Token::Pow);
                } else {
                    tokens.push(Token::Mul);
                }
            }
            value => return Err(format!("unsupported character {value:?}")),
        }
    }
    Ok(tokens)
}

fn push(tokens: &mut Vec<Token>, token: Token, index: &mut usize) {
    tokens.push(token);
    *index += 1;
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    depth: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            index: 0,
            depth: 0,
        }
    }

    fn parse(mut self) -> Result<f64, String> {
        let value = self.expression()?;
        if self.index != self.tokens.len() {
            return Err(format!("unexpected token {:?}", self.tokens[self.index]));
        }
        Ok(value)
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.product()?;
        loop {
            value = match self.peek() {
                Some(Token::Add) => {
                    self.index += 1;
                    value + self.product()?
                }
                Some(Token::Sub) => {
                    self.index += 1;
                    value - self.product()?
                }
                _ => return Ok(value),
            };
        }
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut value = self.power()?;
        loop {
            value = match self.peek() {
                Some(Token::Mul) => {
                    self.index += 1;
                    value * self.power()?
                }
                Some(Token::Div) => {
                    self.index += 1;
                    value / self.power()?
                }
                Some(Token::Mod) => {
                    self.index += 1;
                    value % self.power()?
                }
                _ => return Ok(value),
            };
        }
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.unary()?;
        if self.peek() == Some(&Token::Pow) {
            self.index += 1;
            Ok(base.powf(self.power()?))
        } else {
            Ok(base)
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(Token::Add) => {
                self.index += 1;
                self.unary()
            }
            Some(Token::Sub) => {
                self.index += 1;
                Ok(-self.unary()?)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<f64, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("nesting exceeds 64 levels".into());
        }
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .ok_or("unexpected end")?;
        self.index += 1;
        let result = match token {
            Token::Number(value) => Ok(value),
            Token::Open => {
                let value = self.expression()?;
                self.expect(Token::Close)?;
                Ok(value)
            }
            Token::Name(name) if self.peek() == Some(&Token::Open) => self.call(&name),
            Token::Name(name) if name.eq_ignore_ascii_case("pi") => Ok(std::f64::consts::PI),
            Token::Name(name) if name.eq_ignore_ascii_case("e") => Ok(std::f64::consts::E),
            other => Err(format!("unexpected token {other:?}")),
        };
        self.depth -= 1;
        result
    }

    fn call(&mut self, name: &str) -> Result<f64, String> {
        self.expect(Token::Open)?;
        let mut args = Vec::new();
        if self.peek() != Some(&Token::Close) {
            loop {
                args.push(self.expression()?);
                if self.peek() != Some(&Token::Comma) {
                    break;
                }
                self.index += 1;
            }
        }
        self.expect(Token::Close)?;
        apply(name, &args)
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        if self.peek() == Some(&expected) {
            self.index += 1;
            Ok(())
        } else {
            Err(format!("expected {expected:?}"))
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }
}

fn apply(name: &str, args: &[f64]) -> Result<f64, String> {
    let one = || {
        args.first()
            .copied()
            .ok_or_else(|| format!("{name} needs one argument"))
    };
    match name.to_ascii_lowercase().as_str() {
        "sqrt" => Ok(one()?.sqrt()),
        "abs" => Ok(one()?.abs()),
        "round" => Ok(one()?.round()),
        "floor" => Ok(one()?.floor()),
        "ceil" => Ok(one()?.ceil()),
        "sin" => Ok(one()?.sin()),
        "cos" => Ok(one()?.cos()),
        "tan" => Ok(one()?.tan()),
        "asin" => Ok(one()?.asin()),
        "acos" => Ok(one()?.acos()),
        "atan" => Ok(one()?.atan()),
        "log" => Ok(one()?.ln()),
        "log2" => Ok(one()?.log2()),
        "log10" => Ok(one()?.log10()),
        "exp" => Ok(one()?.exp()),
        "pow" if args.len() == 2 => Ok(args[0].powf(args[1])),
        "min" if !args.is_empty() => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
        "max" if !args.is_empty() => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        _ => Err(format!("unknown function or wrong arity: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safely_parses_math() {
        assert!(evaluate("sqrt(144) + 2^3").contains("20.0"));
        let rejected = evaluate("process.exit()");
        assert!(rejected.contains("invalid expression"));
        assert!(!rejected.contains("\"result\""));
    }
}
