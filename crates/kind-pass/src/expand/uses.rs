use fxhash::FxHashMap;
use kind_report::data::Diagnostic;
use kind_tree::concrete::{visitor::Visitor, Module};
/// Expands sum type and record definitions to a lot of
/// helper definitions like eliminators and replace qualified identifiers
/// by their module names.
use std::sync::mpsc::Sender;

use crate::diagnostic::PassDiagnostic;

pub struct Expand {
    pub names: FxHashMap<String, String>,
    pub errors: Sender<Box<dyn Diagnostic>>,
    pub failed: bool,
}

impl Visitor for Expand {
    fn visit_qualified_ident(&mut self, ident: &mut kind_tree::symbol::QualifiedIdent) {
        if ident.get_aux().is_none() {
            return;
        }
        let alias = match self.names.get(&ident.get_root()) {
            Some(path) => path,
            None => {
                self.errors
                    .send(Box::new(PassDiagnostic::CannotFindAlias(
                        ident.get_root(),
                        ident.range,
                    )))
                    .unwrap();
                self.failed = true;
                return;
            }
        };
        match &ident.get_aux() {
            Some(post) => {
                ident.change_root(format!("{}.{}", alias, post));
                ident.reset_aux()
            }
            None => ident.change_root(alias.clone()),
        }
    }
}

pub fn expand_uses(module: &mut Module, errors: Sender<Box<dyn Diagnostic>>) -> bool {
    let mut session = Expand {
        names: module.uses.clone(),
        errors,
        failed: false,
    };
    for entry in module.entries.iter_mut() {
        session.visit_top_level(entry)
    }
    session.failed
}
