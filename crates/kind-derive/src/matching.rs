//! Module to derive a dependent
//! eliminator out of a sum type declaration.

use fxhash::FxHashMap;
use kind_report::data::Diagnostic;
use kind_span::Range;

use kind_tree::concrete::expr::Expr;
use kind_tree::concrete::pat::{Pat, PatIdent};
use kind_tree::concrete::*;
use kind_tree::concrete::{self};
use kind_tree::symbol::{Ident, QualifiedIdent};

use crate::errors::DeriveError;
use crate::subst::substitute_in_expr;

/// Derives an eliminator from a sum type declaration.
pub fn derive_match(
    range: Range,
    sum: &SumTypeDecl,
) -> (concrete::Entry, Vec<Box<dyn Diagnostic>>) {
    let mut errs: Vec<Box<dyn Diagnostic>> = Vec::new();

    let mk_var = |name: Ident| -> Box<Expr> {
        Box::new(Expr {
            data: ExprKind::Var { name },
            range,
        })
    };

    let mk_cons = |name: QualifiedIdent, args: Vec<Binding>| -> Box<Expr> {
        Box::new(Expr {
            data: ExprKind::Constr { name, args },
            range,
        })
    };

    let mk_app = |fun: Box<Expr>, args: Vec<AppBinding>, range: Range| -> Box<Expr> {
        Box::new(Expr {
            data: ExprKind::App { fun, args },
            range,
        })
    };

    let mk_pi = |name: Ident, typ: Box<Expr>, body: Box<Expr>| -> Box<Expr> {
        Box::new(Expr {
            data: ExprKind::All {
                param: Some(name),
                typ,
                body,
                erased: false,
            },
            range,
        })
    };

    let mk_typ = || -> Box<Expr> {
        Box::new(Expr {
            data: ExprKind::Lit { lit: Literal::Type },
            range,
        })
    };

    let name = sum.name.add_segment("match");

    let mut types = Telescope::default();

    for arg in sum.parameters.iter() {
        types.push(arg.to_implicit())
    }

    for arg in sum.indices.iter() {
        types.push(arg.to_implicit())
    }

    // The type

    let all_args = sum.parameters.extend(&sum.indices);
    let res_motive_ty = mk_cons(
        sum.name.clone(),
        all_args
            .iter()
            .cloned()
            .map(|x| Binding::Positional(mk_var(x.name)))
            .collect(),
    );

    let indice_names: Vec<AppBinding> = sum
        .indices
        .iter()
        .map(|x| AppBinding::explicit(mk_var(x.name.clone())))
        .collect();

    // Sccrutinzies

    types.push(Argument {
        hidden: false,
        erased: false,
        name: Ident::generate("scrutinizer"),
        typ: Some(res_motive_ty.clone()),
        range,
    });

    // Motive with indices

    let motive_ident = Ident::new_static("motive", range);

    let motive_type = sum.indices.iter().rfold(
        mk_pi(Ident::new_static("val_", range), res_motive_ty, mk_typ()),
        |out, arg| {
            mk_pi(
                arg.name.clone(),
                arg.typ.clone().unwrap_or_else(mk_typ),
                out,
            )
        },
    );

    types.push(Argument {
        hidden: false,
        erased: true,
        name: motive_ident.clone(),
        typ: Some(motive_type),
        range,
    });

    let params = sum
        .parameters
        .map(|x| Binding::Positional(mk_var(x.name.clone())));
    let indices = sum
        .indices
        .map(|x| Binding::Positional(mk_var(x.name.clone())));

    // Constructors type
    for cons in &sum.constructors {
        let vars: Vec<Binding> = cons
            .args
            .iter()
            .map(|x| Binding::Positional(mk_var(x.name.clone())))
            .collect();

        let cons_inst = mk_cons(
            sum.name.add_segment(cons.name.to_str()),
            [
                params.as_slice(),
                if cons.typ.is_none() {
                    indices.as_slice()
                } else {
                    &[]
                },
                vars.as_slice(),
            ]
            .concat(),
        );

        let mut indices_of_cons = match cons.typ.clone().map(|x| x.data) {
            Some(ExprKind::Constr { name: _, args }) => {
                let mut new_args = Vec::with_capacity(args.len());
                for arg in &args[sum.parameters.len()..].to_vec() {
                    new_args.push(match arg {
                        Binding::Positional(expr) => AppBinding::explicit(expr.clone()),
                        Binding::Named(range, _, expr) => {
                            errs.push(Box::new(DeriveError::CannotUseNamedVariable(*range)));
                            AppBinding::explicit(expr.clone())
                        }
                    });
                }
                new_args
            }
            _ => [indice_names.as_slice()].concat(),
        };

        indices_of_cons.push(AppBinding::explicit(cons_inst));

        let cons_tipo = mk_app(mk_var(motive_ident.clone()), indices_of_cons, range);

        let args = if cons.typ.is_some() {
            cons.args.clone()
        } else {
            sum.indices.extend(&cons.args)
        };

        let cons_type = args.iter().rfold(cons_tipo, |out, arg| {
            mk_pi(
                arg.name.clone(),
                arg.typ.clone().unwrap_or_else(mk_typ),
                out,
            )
        });

        types.push(Argument {
            hidden: false,
            erased: false,
            name: Ident::new_static(&format!("{}_", cons.name), range),
            typ: Some(cons_type),
            range,
        });
    }

    if !errs.is_empty() {
        let entry = Entry {
            name,
            docs: Vec::new(),
            args: types,
            typ: Box::new(Expr {
                data: ExprKind::Hole,
                range,
            }),
            rules: vec![],
            range,
            attrs: Vec::new(),
            generated_by: Some(sum.name.to_string()),
        };
        return (entry, errs);
    }

    let mut res: Vec<AppBinding> = [indice_names.as_slice()].concat();
    res.push(AppBinding::explicit(mk_var(Ident::generate("scrutinizer"))));
    let ret_ty = mk_app(mk_var(motive_ident.clone()), res, range);

    let mut rules = Vec::new();

    for cons in &sum.constructors {
        let cons_ident = sum.name.add_segment(cons.name.to_str());
        let mut pats: Vec<Box<Pat>> = Vec::new();

        let irrelev: Vec<bool>;
        let spine_params: Vec<Ident>;
        let spine: Vec<Ident>;

        let mut args_indices: Vec<AppBinding>;

        match &cons.typ {
            Some(expr) => match &**expr {
                Expr {
                    data: ExprKind::Constr { args, .. },
                    ..
                } => {
                    irrelev = cons.args.map(|x| x.erased).to_vec();
                    spine_params = sum
                        .parameters
                        .extend(&cons.args)
                        .map(|x| x.name.with_name(|f| format!("{}_", f)))
                        .to_vec();
                    spine = cons
                        .args
                        .map(|x| x.name.with_name(|f| format!("{}_", f)))
                        .to_vec();
                    args_indices = args
                        .iter()
                        .map(|x| match x {
                            Binding::Positional(expr) => AppBinding {
                                erased: false,
                                data: expr.clone(),
                            },
                            Binding::Named(_, _, _) => unreachable!(),
                        })
                        .collect::<Vec<AppBinding>>();
                    args_indices = {
                        let mut indices = args_indices[sum.parameters.len()..].to_vec();

                        let renames = FxHashMap::from_iter(
                            sum.parameters
                                .extend(&cons.args)
                                .map(|x| (x.name.to_string(), format!("{}_", x.name)))
                                .iter()
                                .cloned(),
                        );

                        for indice in &mut indices {
                            substitute_in_expr(&mut indice.data, &renames)
                        }
                        indices
                    };
                }
                _ => unreachable!(),
            },
            None => {
                irrelev = sum.indices.extend(&cons.args).map(|x| x.erased).to_vec();
                spine_params = sum
                    .parameters
                    .extend(&sum.indices)
                    .extend(&cons.args)
                    .map(|x| x.name.with_name(|f| format!("{}_", f)))
                    .to_vec();
                spine = sum
                    .indices
                    .extend(&cons.args)
                    .map(|x| x.name.with_name(|f| format!("{}_", f)))
                    .to_vec();
                args_indices = sum
                    .indices
                    .clone()
                    .map(|x| AppBinding {
                        data: mk_var(x.name.clone()),
                        erased: false,
                    })
                    .to_vec();
            }
        }

        pats.push(Box::new(Pat {
            data: concrete::pat::PatKind::App(
                cons_ident.clone(),
                spine_params
                    .iter()
                    .cloned()
                    .map(|x| {
                        Box::new(Pat {
                            data: concrete::pat::PatKind::Var(PatIdent(x)),
                            range,
                        })
                    })
                    .collect(),
            ),
            range,
        }));

        pats.push(Box::new(Pat {
            data: concrete::pat::PatKind::Var(PatIdent(Ident::generate("motive"))),
            range,
        }));

        for cons2 in &sum.constructors {
            pats.push(Box::new(Pat {
                data: concrete::pat::PatKind::Var(PatIdent(cons2.name.clone())),
                range,
            }));
        }

        let mut args = args_indices.clone();

        args.push(AppBinding {
            data: Box::new(Expr {
                data: ExprKind::Constr {
                    name: cons_ident.clone(),
                    args: spine_params
                        .iter()
                        .cloned()
                        .map(|x| Binding::Positional(mk_var(x)))
                        .collect(),
                },
                range,
            }),
            erased: false,
        });

        let body = Box::new(Expr {
            data: ExprKind::Ann {
                val: mk_app(
                    mk_var(cons.name.clone()),
                    spine
                        .iter()
                        .zip(irrelev)
                        .map(|(arg, erased)| AppBinding {
                            data: mk_var(arg.clone()),
                            erased,
                        })
                        .collect(),
                    cons.name.range,
                ),
                typ: mk_app(mk_var(motive_ident.clone()), args, range),
            },
            range,
        });

        let rule = Box::new(Rule {
            name: name.clone(),
            pats,
            body,
            range: cons.name.range,
        });

        rules.push(rule)
    }
    // Rules

    let entry = Entry {
        name,
        docs: Vec::new(),
        args: types,
        typ: ret_ty,
        rules,
        range,
        attrs: Vec::new(),
        generated_by: Some(sum.name.to_string().clone()),
    };

    (entry, errs)
}