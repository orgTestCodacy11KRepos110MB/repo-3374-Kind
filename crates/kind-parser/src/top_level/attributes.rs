use kind_span::{Locatable, Range};
use kind_tree::concrete::{Attribute, AttributeStyle};

use crate::diagnostic::SyntaxDiagnostic;
use crate::lexer::tokens::Token;
use crate::state::Parser;

impl<'a> Parser<'a> {
    fn parse_attr_args(&mut self) -> Result<(Vec<AttributeStyle>, Range), SyntaxDiagnostic> {
        let mut attrs = Vec::new();

        let mut range = self.range();

        if self.check_and_eat(Token::LBracket) {
            while let Some(res) = self.try_single(&|fun| fun.parse_attr_style())? {
                attrs.push(res);
                if !self.check_and_eat(Token::Comma) {
                    break;
                }
            }
            let start = range;
            range = self.range();
            self.eat_closing_keyword(Token::RBracket, start)?;
        }

        Ok((attrs, range))
    }

    fn parse_attr_style(&mut self) -> Result<AttributeStyle, SyntaxDiagnostic> {
        match self.get().clone() {
            Token::LowerId(_) | Token::UpperId(_, None) => {
                let range = self.range();
                let ident = self.parse_any_id()?;
                Ok(AttributeStyle::Ident(range, ident))
            }
            Token::Num60(num) => {
                let range = self.range();
                self.advance();
                Ok(AttributeStyle::Number(range, num))
            }
            Token::Str(str) => {
                let range = self.range();
                self.advance();
                Ok(AttributeStyle::String(range, str))
            }
            Token::LBracket => {
                let range = self.range();
                self.advance();

                let mut attrs = Vec::new();
                while let Some(res) = self.try_single(&|fun| fun.parse_attr_style())? {
                    attrs.push(res);
                    if !self.check_and_eat(Token::Comma) {
                        break;
                    }
                }

                let end = self.range();
                self.eat_closing_keyword(Token::RBracket, range)?;
                Ok(AttributeStyle::List(range.mix(end), attrs))
            }
            _ => self.fail(Vec::new()),
        }
    }

    fn parse_attr(&mut self) -> Result<Attribute, SyntaxDiagnostic> {
        let start = self.range();
        self.eat_variant(Token::Hash)?;

        let name = self.parse_id()?;

        let (args, mut last) = self.parse_attr_args()?;

        let style = if self.check_and_eat(Token::Eq) {
            let res = self.parse_attr_style()?;
            last = res.locate();
            Some(res)
        } else {
            None
        };
        Ok(Attribute {
            range: start.mix(last),
            value: style,
            args,
            name,
        })
    }

    pub fn parse_attrs(&mut self) -> Result<Vec<Attribute>, SyntaxDiagnostic> {
        let mut attrs = Vec::new();
        while let Some(res) = self.try_single(&|fun| fun.parse_attr())? {
            attrs.push(res);
        }
        Ok(attrs)
    }
}
