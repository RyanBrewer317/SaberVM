/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::header::*;
use std::collections::HashMap;
use std::collections::VecDeque;

/// The type of the stack; a sequence of the types of things on the stack.
pub type StackType = VecDeque<Type>;
/// The type of the compile-time stack, a stack of compile-time values.
pub type CTStackType = Vec<CTStackVal>;
/// The type of the constraints produced by the first pass of the verifier.
/// Note that this is not like Hindley-Milner or anything like that:
/// the first pass gets the types of all the functions without using any constraints.
/// The constraints are only checked to make sure the functions are well-typed, not to derive any types.
pub enum Constraint {
    CallConstraint(Pos, Label, StackType, CTStackType),
    EqConstraint(Pos, Op1, Label, Type),
}
pub type Constraints = Vec<Constraint>;

pub fn go(unverified_stmts: Vec<Stmt1>) -> Result<Vec<Stmt2>, Error> {
    let mut verified_stmts: Vec<Stmt2> = vec![];
    let stmts2: Vec<(Stmt2, Constraints)> = unverified_stmts
        .iter()
        .map(|stmt| first_pass(stmt))
        .collect::<Result<Vec<_>, Error>>()?;
    let mut constraints: Constraints = vec![];
    for pair in stmts2 {
        let (stmt, c) = pair;
        constraints.extend(c);
        verified_stmts.push(stmt);
    }
    match verified_stmts.get(0) {
        Some(Stmt2::Func(_, Type::Func(param_ts), _)) => {
            if param_ts.len() != 0 {
                return Err(Error::TypeErrorMainHasArgs);
            }
        }
        _ => (),
    }
    let () = second_pass(
        constraints,
        &verified_stmts
            .iter()
            .map(|Stmt2::Func(l, t, _)| (l.to_owned(), t.to_owned()))
            .collect(),
    )?;
    Ok(verified_stmts)
}

pub fn first_pass(stmt: &Stmt1) -> Result<(Stmt2, Constraints), Error> {
    let Stmt1::Func(label, ops) = stmt;
    let mut ops_iter = ops.iter();

    // The stacks used for this pass algorithm.
    let mut compile_time_stack: CTStackType = vec![];
    let mut stack_type: StackType = VecDeque::from([]);
    let mut quantification_stack: Vec<Quantification> = vec![];

    // The types the function expects the top of the stack to have.
    let mut param_types: Vec<Type> = vec![];

    // The constraints generated by this first pass.
    let mut constraints: Constraints = vec![];
    // The verified bytecode produced by this first pass.
    let mut verified_ops: Vec<Op2> = vec![];

    // The list of region variables the function is quantified (polymorphic) over.
    let mut rgn_vars: Vec<Region> = vec![];
    // The list of type variables the function is quantified (polymorphic) over.
    let mut type_vars: Vec<Id> = vec![];

    // The kind context (\Delta in the Capability Calculus paper) the function has.
    // This is generated alongside the above three variables, capturing the same information in a different form
    // that's useful for different purposes.
    let mut kind_context: Vec<KindContextEntry> = vec![];

    // The generator of fresh identifiers.
    let mut fresh_id: u32 = 0;
    // The variable tracking the current byte position, for nice error reporting.
    let mut pos = *label;

    let mut next_region_is_unique = false;

    loop {
        match ops_iter.next() {
            None => break,
            Some(op) => match op {
                Op1::Req => match compile_time_stack.pop() {
                    Some(CTStackVal::Type(t)) => {
                        param_types.push(t.clone());
                        stack_type.push_front(t);
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::Region => {
                    let id = Id(*label, fresh_id);
                    let r = Region {
                        unique: next_region_is_unique,
                        id: id,
                    };
                    next_region_is_unique = false;
                    compile_time_stack.push(CTStackVal::Region(r.clone()));
                    rgn_vars.push(r.clone());
                    kind_context.push(KindContextEntry::Region(r));
                    fresh_id += 1;
                }
                Op1::Unique => {
                    next_region_is_unique = true;
                }
                Op1::Handle => match compile_time_stack.pop() {
                    Some(CTStackVal::Region(r)) => {
                        compile_time_stack.push(CTStackVal::Type(Type::Handle(r)));
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Region, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::I32 => compile_time_stack.push(CTStackVal::Type(Type::I32)),
                Op1::Tuple(n) => {
                    let mut ts = vec![];
                    for _ in 0..*n {
                        match compile_time_stack.pop() {
                            Some(CTStackVal::Type(t)) => ts.push(t),
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Type, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        }
                    }
                    match compile_time_stack.pop() {
                        Some(CTStackVal::Region(r)) => {
                            compile_time_stack.push(CTStackVal::Type(Type::Tuple(ts, r)))
                        }
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Region, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    }
                }
                Op1::Quantify => match compile_time_stack.pop() {
                    Some(CTStackVal::Size(s)) => {
                        let id = Id(*label, fresh_id);
                        fresh_id += 1;
                        compile_time_stack.push(CTStackVal::Type(Type::Var(id.clone(), s)));
                        type_vars.push(id.clone());
                        kind_context.push(KindContextEntry::Type(id));
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Size, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::Some => match compile_time_stack.pop() {
                    Some(CTStackVal::Size(s)) => {
                        let id = Id(*label, fresh_id);
                        fresh_id += 1;
                        compile_time_stack.push(CTStackVal::Type(Type::Var(id.clone(), s)));
                        quantification_stack.push(Quantification::Exist(id, s));
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Size, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::All => match compile_time_stack.pop() {
                    Some(CTStackVal::Size(s)) => {
                        let id = Id(*label, fresh_id);
                        fresh_id += 1;
                        compile_time_stack.push(CTStackVal::Type(Type::Var(id.clone(), s)));
                        quantification_stack.push(Quantification::Forall(id, s));
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Size, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::Rgn => {
                    let id = Id(*label, fresh_id);
                    let r = Region {
                        unique: next_region_is_unique,
                        id: id,
                    };
                    fresh_id += 1;
                    compile_time_stack.push(CTStackVal::Region(r.clone()));
                    quantification_stack.push(Quantification::Region(r));
                }
                Op1::End => match quantification_stack.pop() {
                    Some(Quantification::Exist(id, s)) => match compile_time_stack.pop() {
                        Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                            Some(CTStackVal::Type(Type::Var(id2, _))) if id == id2 => {
                                compile_time_stack.push(CTStackVal::Type(Type::Exists(
                                    id,
                                    s,
                                    Box::new(t),
                                )))
                            }
                            Some(CTStackVal::Type(Type::Var(id2, _))) => {
                                return Err(Error::TypeErrorSpecificTypeVarExpected(
                                    pos, *op, id, id2,
                                ))
                            }
                            Some(CTStackVal::Type(t)) => {
                                return Err(Error::TypeErrorTypeVarExpected(pos, *op, id, t))
                            }
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Type, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        },
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    Some(Quantification::Forall(id, s)) => match compile_time_stack.pop() {
                        Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                            Some(CTStackVal::Type(Type::Var(id2, _))) if id == id2 => {
                                compile_time_stack.push(CTStackVal::Type(Type::Forall(
                                    id,
                                    s,
                                    Box::new(t),
                                )))
                            }
                            Some(CTStackVal::Type(Type::Var(id2, _))) => {
                                return Err(Error::TypeErrorSpecificTypeVarExpected(
                                    pos, *op, id, id2,
                                ))
                            }
                            Some(CTStackVal::Type(t)) => {
                                return Err(Error::TypeErrorTypeVarExpected(pos, *op, id, t))
                            }
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Type, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        },
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    Some(Quantification::Region(r)) => match compile_time_stack.pop() {
                        Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                            Some(CTStackVal::Region(r2)) if r.id == r2.id => compile_time_stack
                                .push(CTStackVal::Type(Type::ForallRegion(r, Box::new(t)))),
                            Some(CTStackVal::Region(r2)) => {
                                return Err(Error::RegionError(pos, *op, r, r2))
                            }
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Region, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        },
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    None => return Err(Error::TypeErrorEmptyQuantificationStack(pos, *op)),
                },
                Op1::Func(n) => {
                    let mut ts = vec![];
                    for _ in 0..*n {
                        match compile_time_stack.pop() {
                            Some(CTStackVal::Type(t)) => ts.push(t),
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Type, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        }
                    }
                    compile_time_stack.push(CTStackVal::Type(Type::Func(ts)))
                }
                Op1::CTGet(i) => {
                    match compile_time_stack.get(compile_time_stack.len() - 1 - (*i) as usize) {
                        Some(ctval) => compile_time_stack.push(ctval.clone()),
                        None => {
                            return Err(Error::TypeErrorCTGetOutOfRange(
                                pos,
                                *i,
                                compile_time_stack.len(),
                            ))
                        }
                    }
                }
                Op1::Unpack => match compile_time_stack.pop() {
                    Some(CTStackVal::Type(Type::Exists(_id, _s, t))) => {
                        compile_time_stack.push(CTStackVal::Type(*t))
                    }
                    Some(CTStackVal::Type(t)) => {
                        return Err(Error::TypeErrorExistentialExpected(pos, *op, t))
                    }
                    Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::Get(i) => {
                    let stack_len = stack_type.len();
                    if stack_len == 0 {
                        return Err(Error::TypeErrorEmptyStack(pos, *op));
                    }
                    let i2 = usize::from(*i);
                    if stack_len - 1 < i2 {
                        return Err(Error::TypeErrorGetOutOfRange(pos, *i, stack_len));
                    }
                    let mut offset = 0;
                    for j in 0..*i {
                        offset += stack_type[stack_len - 1 - (j as usize)].size();
                    }
                    let t = stack_type.get(stack_len - 1 - i2).unwrap().clone();
                    let size = t.size();
                    stack_type.push_back(t);
                    verified_ops.push(Op2::Get(offset, size));
                }
                Op1::Init(i) => {
                    let mb_val = stack_type.pop_back();
                    let mb_tpl = stack_type.pop_back();
                    match mb_tpl {
                        Some(tpl) => match tpl.clone() {
                            // TODO: check that r is in the regions declared by the function (which is necessary, right?)
                            Type::Tuple(component_types, r) => {
                                match component_types.get(usize::from(*i)) {
                                    None => {
                                        return Err(Error::TypeErrorInitOutOfRange(
                                            pos,
                                            *i,
                                            component_types.len(),
                                        ))
                                    }
                                    Some(formal) => match mb_val {
                                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                                        Some(actual) => {
                                            let success = || {
                                                stack_type.push_back(tpl);
                                                let mut offset = 0;
                                                for i2 in 0..*i {
                                                    offset += component_types[i2 as usize].size()
                                                }
                                                verified_ops.push(Op2::Init(offset, actual.size()));
                                            };
                                            if type_eq(formal, &actual) {
                                                success();
                                            } else if let Type::Guess(l) = actual {
                                                constraints.push(Constraint::EqConstraint(
                                                    pos,
                                                    *op,
                                                    l,
                                                    formal.clone(),
                                                ));
                                                success();
                                            } else {
                                                return Err(Error::TypeErrorInitTypeMismatch(
                                                    pos,
                                                    formal.clone(),
                                                    actual,
                                                ));
                                            }
                                        }
                                    },
                                }
                            }
                            _ => return Err(Error::TypeErrorTupleExpected(pos, *op, tpl)),
                        },
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                }
                Op1::Malloc => {
                    let mb_type = compile_time_stack.pop();
                    let t = match mb_type {
                        Some(CTStackVal::Type(t)) => t,
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    };
                    let mb_rgn_handle = stack_type.pop_back();
                    match mb_rgn_handle {
                        Some(Type::Handle(r)) => {
                            // check that t is in r and that r is in the list of declared regions
                            let size = t.size();
                            stack_type.push_back(t);
                            verified_ops.push(Op2::Malloc(size));
                        }
                        Some(t) => {
                            return Err(Error::TypeErrorRegionHandleExpected(pos, *op, t));
                        }
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                }
                Op1::Proj(i) => {
                    let mb_tpl = stack_type.pop_back();
                    match mb_tpl {
                        Some(tpl) => match tpl {
                            Type::Tuple(component_types, r) => {
                                // TODO: add check that r is in the list of declared regions
                                match component_types.get(usize::from(*i)) {
                                    None => {
                                        return Err(Error::TypeErrorProjOutOfRange(
                                            pos,
                                            *i,
                                            component_types.len(),
                                        ))
                                    }
                                    Some(t) => {
                                        let mut offset = 0;
                                        for i2 in 0..*i {
                                            offset += component_types[i2 as usize].size();
                                        }
                                        stack_type.push_back(t.clone());
                                        verified_ops.push(Op2::Proj(offset, t.size()));
                                    }
                                }
                            }
                            t => return Err(Error::TypeErrorTupleExpected(pos, *op, t)),
                        },
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                }
                Op1::Call => {
                    let mb_type = stack_type.pop_back();
                    match mb_type {
                        Some(t) => match t {
                            Type::Guess(label) => {
                                constraints.push(Constraint::CallConstraint(
                                    pos,
                                    label,
                                    stack_type.to_owned(),
                                    compile_time_stack.to_owned(),
                                ));
                            }
                            Type::Func(args) => {
                                let () = verify_call(
                                    pos,
                                    &args,
                                    stack_type.to_owned(),
                                )?;
                            }
                            // TODO: polymorphic function types
                            _ => return Err(Error::TypeErrorFunctionExpected(pos, *op, t)),
                        },
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                    verified_ops.push(Op2::Call)
                }
                Op1::Print => {
                    todo!()
                }
                Op1::Lit(lit) => {
                    todo!()
                }
                Op1::GlobalFunc(label) => {
                    todo!()
                }
                Op1::Halt => {
                    todo!()
                }
                Op1::Pack => {
                    todo!()
                }
                Op1::Size(s) => {
                    todo!()
                }
                Op1::NewRgn => {
                    todo!()
                }
                Op1::FreeRgn => {
                    todo!()
                }
            },
        }
        pos += 1;
    }
    if quantification_stack.len() > 0 {
        return Err(Error::TypeErrorNonEmptyQuantificationStack(*label));
    }
    let t = Type::Func(param_types);
    // wrap t in the quantifiers from kind_context
    Ok((Stmt2::Func(*label, t, verified_ops), constraints))
}

pub fn second_pass(constraints: Constraints, types: &HashMap<Label, Type>) -> Result<(), Error> {
    todo!()
}

/// Perform some variable substitutions within a type.
/// This does not modify the original.
pub fn substitute_t(typ: &Type, tsubs: &HashMap<Id, Type>, rsubs: &HashMap<Id, Region>) -> Type {
    match typ {
        Type::I32 => Type::I32,
        Type::Handle(r) => Type::Handle(substitute_r(r, rsubs)),
        Type::Tuple(ts, r) => Type::Tuple(
            ts.iter().map(|t| substitute_t(t, tsubs, rsubs)).collect(),
            substitute_r(r, rsubs),
        ),
        Type::Var(id, repr) => match tsubs.get(id) {
            Some(new) => new.clone(),
            None => Type::Var(*id, repr.clone()),
        },
        Type::Func(args) => {
            Type::Func(args.iter().map(|t| substitute_t(t, tsubs, rsubs)).collect())
        }
        Type::Exists(id, s, t) => Type::Exists(*id, *s, Box::new(substitute_t(t, tsubs, rsubs))),
        Type::Forall(id, s, t) => Type::Forall(*id, *s, Box::new(substitute_t(t, tsubs, rsubs))),
        Type::ForallRegion(id, t) => {
            Type::ForallRegion(*id, Box::new(substitute_t(t, tsubs, rsubs)))
        }
        Type::Guess(label) => Type::Guess(*label),
    }
}

/// Perform some variable substitutions in a compile-time region value.
/// This does not modify the original
pub fn substitute_r(r: &Region, rsubs: &HashMap<Id, Region>) -> Region {
    match rsubs.get(&r.id) {
        Some(r2) => *r2,
        None => *r,
    }
}

/// Check if two types are equal, for typechecking purposes.
pub fn type_eq(type1: &Type, type2: &Type) -> bool {
    match (type1, type2) {
        (Type::I32, Type::I32) => true,
        (Type::Handle(r1), Type::Handle(r2)) => r1 == r2,
        (Type::Tuple(ts1, r1), Type::Tuple(ts2, r2)) => {
            r1 == r2 && ts1.len() == ts2.len() && {
                let mut ts2 = ts2.iter();
                for t1 in ts1 {
                    let t2 = ts2.next().unwrap();
                    if !type_eq(t1, t2) {
                        return false;
                    }
                }
                return true;
            }
        }
        (Type::Var(id1, repr1), Type::Var(id2, repr2)) => id1 == id2 && repr1 == repr2,
        (Type::Func(ts1), Type::Func(ts2)) => {
            ts1.iter().zip(ts2.iter()).all(|(t1, t2)| type_eq(&t1, &t2))
        }
        (Type::Exists(id1, repr1, t1), Type::Exists(id2, repr2, t2)) => {
            let mut sub = HashMap::new();
            sub.insert(*id2, Type::Var(*id1, repr1.clone()));
            let t2_subbed = substitute_t(t2, &sub, &HashMap::new());
            // dbg!(pretty::typ(&t1), pretty::typ(&t2_subbed));
            repr1 == repr2 && type_eq(t1, &t2_subbed)
        }
        (Type::Guess(label1), Type::Guess(label2)) => label1 == label2,
        (_, _) => false,
    }
}

pub fn verify_call(
    pos: Pos,
    args: &Vec<Type>,
    mut stack_type: StackType,
) -> Result<(), Error> {
    let arg_ts_needed = args;
    let mut arg_ts_present = vec![];
    for _ in 0..arg_ts_needed.len() {
        match stack_type.pop_back() {
            Some(t) => arg_ts_present.push(t.clone()),
            None => {
                return Err(Error::TypeErrorNotEnoughRuntimeArgs(
                    pos,
                    arg_ts_needed.len(),
                    arg_ts_present.len(),
                ));
            }
        }
    }
    let types_match = arg_ts_present
        .iter()
        .zip(arg_ts_needed.iter())
        .all(|(t1, t2)| type_eq(t1, t2));
    if !types_match {
        return Err(Error::TypeErrorCallArgTypesMismatch(
            pos,
            arg_ts_needed.to_vec(),
            arg_ts_present,
        ));
    }
    Ok(())
}
