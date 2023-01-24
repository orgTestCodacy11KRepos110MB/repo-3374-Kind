//! Lexes some types of literals. It's a isolated
//! module because it requires a lot of code to
//! parse some specific things like escaped characters
//! inside of strings.

use kind_span::Range;

use crate::diagnostic::{EncodeSequence, SyntaxDiagnostic};
use crate::lexer::tokens::Token;
use crate::Lexer;

impl<'a> Lexer<'a> {
    /// Lex a sequence of digits of the base @base@ with
    /// maximum length of @size@ and turns it into a char.
    fn lex_char_encoded(
        &mut self,
        start: usize,
        size: usize,
        base: u32,
        err: EncodeSequence,
    ) -> Result<char, SyntaxDiagnostic> {
        let string = self.next_chars(size);
        let to_chr = string.and_then(|x| u32::from_str_radix(x, base).ok());
        if let Some(chr) = to_chr.and_then(char::from_u32) {
            return Ok(chr);
        }
        Err(SyntaxDiagnostic::InvalidEscapeSequence(
            err,
            self.mk_range(start),
        ))
    }

    /// Turns a escaped char into a normal char.
    fn lex_escaped_char(&mut self, start: usize) -> Result<char, SyntaxDiagnostic> {
        match self.peekable.peek() {
            None => Err(SyntaxDiagnostic::UnfinishedString(
                self.mk_one_column_range(start),
            )),
            Some(&x) => {
                self.next_char();
                match x {
                    '\'' => Ok('\''),
                    '\"' => Ok('\"'),
                    'n' => Ok('\n'),
                    'r' => Ok('\r'),
                    't' => Ok('\t'),
                    '0' => Ok('\0'),
                    '\\' => Ok('\\'),
                    'x' => self.lex_char_encoded(start, 2, 16, EncodeSequence::Hexa),
                    'u' => self.lex_char_encoded(start, 4, 16, EncodeSequence::Unicode),
                    other => Ok(other),
                }
            }
        }
    }

    /// Lexes a number of base @base@, figuring out it's type
    /// Lexes 0 if not at a digit position
    fn lex_num_and_type_with_base(
        &mut self,
        num_start: usize,
        base: u32,
        err: EncodeSequence,
    ) -> (Token, Range) {
        let num = self.accumulate_while(&|x| x.is_digit(base) || x == '_');
        let num = if num.is_empty() { "0" } else { num };
        let num = num.to_string();
        let type_start = self.span();
        let make_num_err = |x: &Self| {
            (
                Token::Error(Box::new(SyntaxDiagnostic::InvalidNumberRepresentation(
                    err,
                    x.mk_range(num_start),
                ))),
                x.mk_range(num_start),
            )
        };
        match self.peekable.peek() {
            Some('n' | 'N') => {
                self.next_char();
                if let Ok(res) = u128::from_str_radix(&num.replace('_', ""), base) {
                    (Token::Nat(res), self.mk_range(num_start))
                } else {
                    make_num_err(self)
                }
            }
            Some('U' | 'u') => {
                self.next_char();
                let type_ = self.accumulate_while(&|x| x.is_ascii_digit());
                match type_ {
                    "60" => {
                        if let Ok(res) = u64::from_str_radix(&num.replace('_', ""), base) {
                            (Token::Num60(res), self.mk_range(num_start))
                        } else {
                            make_num_err(self)
                        }
                    }
                    "120" => {
                        if let Ok(res) = u128::from_str_radix(&num.replace('_', ""), base) {
                            (Token::Num120(res), self.mk_range(num_start))
                        } else {
                            make_num_err(self)
                        }
                    }
                    _ => (
                        Token::Error(Box::new(SyntaxDiagnostic::InvalidNumberType(
                            format!("u{}", type_),
                            self.mk_range(type_start),
                        ))),
                        self.mk_range(type_start),
                    ),
                }
            }
            Some(_) | None => {
                if let Ok(start) = u64::from_str_radix(&num.replace('_', ""), base) {
                    if let Some('.') = self.peekable.peek() {
                        self.next_char();
                        let num = self.accumulate_while(&|x| x.is_digit(base) || x == '_');
                        let Ok(decimal) = u64::from_str_radix(&num.replace('_', ""), base) else { return make_num_err(self) };
                        (Token::Float(start, decimal), self.mk_range(num_start))
                    } else {
                        (Token::Num60(start), self.mk_range(num_start))
                    }
                } else {
                    make_num_err(self)
                }
            }
        }
    }

    /// Lex numbers with decimal, hexadecimal, binary or octal.
    pub fn lex_number(&mut self) -> (Token, Range) {
        let start = self.span();
        match self.peekable.peek() {
            None => (Token::Eof, self.mk_range(start)),
            Some('0') => {
                self.next_char();
                match self.peekable.peek() {
                    Some('x' | 'X') => {
                        self.next_char();
                        self.lex_num_and_type_with_base(start, 16, EncodeSequence::Hexa)
                    }
                    Some('o' | 'O') => {
                        self.next_char();
                        self.lex_num_and_type_with_base(start, 8, EncodeSequence::Octal)
                    }
                    Some('b' | 'B') => {
                        self.next_char();
                        self.lex_num_and_type_with_base(start, 2, EncodeSequence::Binary)
                    }
                    Some('0'..='9' | _) | None => {
                        self.lex_num_and_type_with_base(start, 10, EncodeSequence::Decimal)
                    }
                }
            }
            Some('0'..='9' | _) => {
                self.lex_num_and_type_with_base(start, 10, EncodeSequence::Decimal)
                
            }
        }
    }

    pub fn lex_char(&mut self) -> Result<char, SyntaxDiagnostic> {
        let start = self.span();
        if let Some(&x) = self.peekable.peek() {
            let chr_start = self.span();
            match x {
                '\\' => {
                    self.next_char();
                    match self.lex_escaped_char(chr_start) {
                        Ok(x) => Ok(x),
                        Err(t) => Err(t),
                    }
                }
                x => {
                    self.next_char();
                    Ok(x)
                }
            }
        } else {
            Err(SyntaxDiagnostic::UnfinishedChar(self.mk_range(start)))
        }
    }
    /// Lexes a string that starts with '"' and ends with the
    /// same char. each string item can contain a escaped char
    /// and if the esaped char is not well-formed then it will
    /// acummulate the error until the end of the string.
    /// TODO: Accumulate multiple encoding errors?
    pub fn lex_string(&mut self) -> (Token, Range) {
        let start = self.span();

        self.next_char();

        let mut string = String::new();
        let mut error: Option<(Token, Range)> = None;

        while let Some(&x) = self.peekable.peek() {
            let chr_start = self.span();
            match x {
                '\"' => break,
                '\\' => {
                    self.next_char();
                    match self.lex_escaped_char(chr_start) {
                        Ok(x) => string.push(x),
                        Err(t) => {
                            self.accumulate_while(&|x| x != '"');
                            error = Some((Token::Error(Box::new(t)), self.mk_range(start)));
                        }
                    }
                    continue;
                }
                x => string.push(x),
            }
            // FIXME: Not sure if it causes a bug!
            self.next_char();
        }

        match (self.next_char(), error) {
            (_, Some(err)) => err,
            (Some('"'), _) => (Token::Str(string), self.mk_range(start)),
            _ => (
                Token::Error(Box::new(SyntaxDiagnostic::UnfinishedString(
                    self.mk_one_column_range(start),
                ))),
                self.mk_range(start),
            ),
        }
    }
}
