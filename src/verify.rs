/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use crate::header::*;
use std::collections::HashMap;

pub fn go(
    types_instrs: Vec<ForwardDec>,
    unverified_stmts: Vec<Stmt1>,
) -> Result<Vec<Stmt2>, Error> {
    let mut types = HashMap::new();
    let mut fresh_id = 0;
    for stmt in types_instrs {
        match type_pass(&stmt, fresh_id) {
            Ok((l, t, new_fresh_id)) => {
                types.insert(l, t);
                fresh_id = new_fresh_id;
            }
            Err(e) => return Err(e),
        }
    }
    let verified_stmts: Vec<Stmt2> = unverified_stmts
        .iter()
        .map(|stmt| definition_pass(stmt, &types, fresh_id))
        .collect::<Result<Vec<_>, Error>>()?;
    match verified_stmts.get(0) {
        Some(Stmt2::Func(_, Type::Func(param_ts), _)) => {
            if param_ts.len() != 0 {
                return Err(Error::TypeErrorMainHasArgs);
            }
        }
        _ => (),
    }
    Ok(verified_stmts)
}

pub fn type_pass(stmt: &ForwardDec, mut fresh_id: u32) -> Result<(Label, Type, u32), Error> {
    let ForwardDec::Func(label, ops) = stmt;
    let mut next_region_is_unique = false;
    let mut compile_time_stack: Vec<CTStackVal> = vec![];
    let mut quantification_stack: Vec<Quantification> = vec![];
    let mut pos = *label;
    for op in ops {
        match op {
            Op1::Unique => next_region_is_unique = true,
            Op1::Handle => handle_handle(pos, op, &mut compile_time_stack)?,
            Op1::I32 => compile_time_stack.push(CTStackVal::Type(Type::I32)),
            Op1::Tuple(n) => handle_tuple(n, pos, op, &mut compile_time_stack)?,
            Op1::Some => handle_some(
                pos,
                op,
                &mut compile_time_stack,
                &mut fresh_id,
                label,
                &mut quantification_stack,
            )?,
            Op1::All => handle_all(
                pos,
                op,
                &mut compile_time_stack,
                &mut fresh_id,
                label,
                &mut quantification_stack,
            )?,
            Op1::Rgn => handle_rgn(
                &mut next_region_is_unique,
                label,
                &mut fresh_id,
                &mut compile_time_stack,
                &mut quantification_stack,
            )?,
            Op1::End => handle_end(pos, op, &mut compile_time_stack, &mut quantification_stack)?,
            Op1::Func(n) => handle_func(n, pos, op, &mut compile_time_stack)?,
            Op1::CTGet(i) => handle_ctget(pos, i, &mut compile_time_stack)?,
            Op1::Size(s) => compile_time_stack.push(CTStackVal::Size((*s).try_into().unwrap())),
            Op1::Ptr => handle_ptr(pos, op, &mut compile_time_stack)?,
            _ => panic!(),
        }
        pos += 1;
    }
    match &compile_time_stack[..] {
        [CTStackVal::Type(t)] => Ok((*label, t.clone(), pos)),
        _ => panic!(),
    }
}

pub fn definition_pass(
    stmt: &Stmt1,
    types: &HashMap<Label, Type>,
    mut fresh_id: u32,
) -> Result<Stmt2, Error> {
    let Stmt1::Func(label, ops) = stmt;
    let mut ops_iter = ops.iter();

    let Some(my_type) = types.get(label).cloned() else {
        panic!("Type not found for label {}", label);
    };
    // The stacks used for this pass algorithm.
    let (mut compile_time_stack, mut stack_type) = setup_verifier(&my_type)?;
    compile_time_stack.reverse();
    let mut quantification_stack: Vec<Quantification> = vec![];

    // The verified bytecode produced by this first pass.
    let mut verified_ops: Vec<Op2> = vec![];

    // The list of region variables the function is quantified (polymorphic) over.
    let mut rgn_vars: Vec<Region> = vec![];
    for ctval in &compile_time_stack {
        if let CTStackVal::Region(r) = ctval {
            rgn_vars.push(r.clone());
        }
    }

    // The variable tracking the current byte position, for nice error reporting.
    let mut pos = *label;

    let mut next_region_is_unique = false;

    loop {
        match ops_iter.next() {
            None => break,
            Some(op) => match op {
                Op1::Unique => next_region_is_unique = true,
                Op1::Handle => handle_handle(pos, op, &mut compile_time_stack)?,
                Op1::I32 => compile_time_stack.push(CTStackVal::Type(Type::I32)),
                Op1::Tuple(n) => handle_tuple(n, pos, op, &mut compile_time_stack)?,
                Op1::Some => handle_some(
                    pos,
                    op,
                    &mut compile_time_stack,
                    &mut fresh_id,
                    label,
                    &mut quantification_stack,
                )?,
                Op1::All => handle_all(
                    pos,
                    op,
                    &mut compile_time_stack,
                    &mut fresh_id,
                    label,
                    &mut quantification_stack,
                )?,
                Op1::Rgn => handle_rgn(
                    &mut next_region_is_unique,
                    label,
                    &mut fresh_id,
                    &mut compile_time_stack,
                    &mut quantification_stack,
                )?,
                Op1::End => handle_rgn(
                    &mut next_region_is_unique,
                    label,
                    &mut fresh_id,
                    &mut compile_time_stack,
                    &mut quantification_stack,
                )?,
                Op1::App => match compile_time_stack.pop() {
                    Some(CTStackVal::Type(t_arg)) => match stack_type.pop() {
                        Some(Type::Forall(id, s, t)) => {
                            if s != t_arg.size() {
                                return Err(Error::SizeError(pos, *op, s, t_arg.size()));
                            }
                            let new_t =
                                substitute_t(&*t, &HashMap::from([(id, t_arg)]), &HashMap::new());
                            stack_type.push(new_t);
                        }
                        Some(t) => return Err(Error::TypeErrorForallExpected(pos, *op, t)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    Some(CTStackVal::Region(r_arg)) => match stack_type.pop() {
                        Some(Type::ForallRegion(r, t, captured_rgns)) => {
                            if r.unique && captured_rgns.iter().any(|r2| r_arg.id == r2.id) {
                                return Err(Error::RegionAccessError(pos, *op, r_arg));
                            }
                            let new_t =
                                substitute_t(&*t, &HashMap::new(), &HashMap::from([(r.id, r_arg)]));
                            stack_type.push(new_t);
                        }
                        Some(t) => return Err(Error::TypeErrorForallRegionExpected(pos, *op, t)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    Some(ctval) => return Err(Error::KindErrorBadApp(pos, *op, ctval)),
                    None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                },
                Op1::Func(n) => handle_func(n, pos, op, &mut compile_time_stack)?,
                Op1::CTGet(i) => handle_ctget(pos, i, &mut compile_time_stack)?,
                Op1::Lced => panic!("Lced should not appear in this context"),
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
                    stack_type.push(t);
                    verified_ops.push(Op2::Get(offset, size));
                }
                Op1::Init(i) => {
                    let mb_val = stack_type.pop();
                    let mb_tpl = stack_type.pop();
                    let f = |component_types: Vec<(bool, Type)>,
                             g: &dyn Fn(
                        &Type,
                        Vec<(bool, Type)>,
                        &mut Vec<Type>,
                        &mut Vec<Op2>,
                    ) -> ()| {
                        match component_types.get(usize::from(*i)) {
                            None => {
                                return Err(Error::TypeErrorInitOutOfRange(
                                    pos,
                                    *i,
                                    component_types.len(),
                                ))
                            }
                            Some((false, formal)) => match mb_val {
                                None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                                Some(actual) => {
                                    if type_eq(formal, &actual) {
                                        g(
                                            &actual,
                                            component_types,
                                            &mut stack_type,
                                            &mut verified_ops,
                                        );
                                    } else {
                                        return Err(Error::TypeErrorInitTypeMismatch(
                                            pos,
                                            formal.clone(),
                                            actual,
                                        ));
                                    }
                                }
                            },
                            Some((true, _t)) => {
                                return Err(Error::TypeErrorDoubleInit(pos, *op, *i))
                            }
                        }
                        Ok(())
                    };
                    match mb_tpl {
                        Some(Type::Tuple(component_types)) => f(
                            component_types,
                            &|actual: &Type,
                              mut component_types: Vec<(bool, Type)>,
                              stack_type: &mut Vec<Type>,
                              verified_ops: &mut Vec<Op2>| {
                                let mut offset = 0;
                                let tpl_size = component_types.iter().map(|(_, t)| t.size()).sum();
                                for i2 in 0..*i {
                                    let (_, t) = &component_types[i2 as usize];
                                    offset += t.size();
                                }
                                component_types[*i as usize] = (true, actual.clone());
                                stack_type.push(Type::Tuple(component_types));
                                verified_ops.push(Op2::Init(offset, actual.size(), tpl_size));
                            },
                        )?,
                        Some(Type::Ptr(boxed_t, r)) => match *boxed_t {
                            Type::Tuple(component_types) => {
                                if rgn_vars.iter().all(|r2| r.id != r2.id) {
                                    return Err(Error::RegionAccessError(pos, *op, r));
                                }
                                f(component_types, &|actual: &Type, mut component_types: Vec<(bool, Type)>, stack_type: &mut Vec<Type>, verified_ops: &mut Vec<Op2>| {
                                    let mut offset = 0;
                                    for i2 in 0..*i {
                                        let (_, t) = &component_types[i2 as usize];
                                        offset += t.size();
                                    }
                                    component_types[*i as usize] = (true, actual.clone());
                                    stack_type.push(Type::Ptr(Box::new(Type::Tuple(component_types)), r));
                                    verified_ops
                                        .push(Op2::InitIP(offset, actual.size()));
                                })?
                            }
                            t => return Err(Error::TypeErrorTupleExpected(pos, *op, t)),
                        },
                        Some(t) => return Err(Error::TypeErrorTupleExpected(pos, *op, t)),
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                }
                Op1::Malloc => {
                    let mb_type = compile_time_stack.pop();
                    match mb_type {
                        Some(CTStackVal::Type(Type::Ptr(t, r))) => {
                            match stack_type.pop() {
                                Some(Type::Handle(r2)) => {
                                    // check that t is in r and that r is in the list of declared regions
                                    if r.id != r2.id {
                                        return Err(Error::RegionError(pos, *op, r, r2));
                                    }
                                    if rgn_vars.iter().all(|r2: &Region| r.id != r2.id) {
                                        return Err(Error::RegionAccessError(pos, *op, r));
                                    }
                                    let t = *t;
                                    let size = t.size();
                                    if let Type::Tuple(component_types) = t {
                                        let mut ts = vec![];
                                        for (_, t) in component_types {
                                            ts.push((false, t));
                                        }
                                        stack_type.push(Type::Ptr(Box::new(Type::Tuple(ts)), r));
                                        verified_ops.push(Op2::Malloc(size));
                                    } else {
                                        return Err(Error::TypeErrorMallocNonTuple(pos, *op, t));
                                    }
                                }
                                Some(t) => {
                                    return Err(Error::TypeErrorRegionHandleExpected(pos, *op, t));
                                }
                                None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                            }
                        }
                        Some(CTStackVal::Type(Type::Tuple(component_types))) => {
                            let mut ts = vec![];
                            for (_, t) in component_types {
                                ts.push((false, t))
                            }
                            let t = Type::Tuple(ts);
                            let size = t.size();
                            stack_type.push(t);
                            verified_ops.push(Op2::Alloca(size));
                        }
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    };
                }
                Op1::Proj(i) => {
                    let mb_tpl = stack_type.pop();
                    let mut f = |component_types: Vec<(bool, Type)>,
                                 g: &dyn Fn(
                        &Type,
                        usize,
                        &mut Vec<Type>,
                        &mut Vec<Op2>,
                        Vec<(bool, Type)>,
                    ) -> ()| {
                        let s: usize = component_types.iter().map(|(_, t)| t.size()).sum();
                        let mb_t = component_types.get(usize::from(*i)).cloned();
                        match mb_t {
                            None => {
                                return Err(Error::TypeErrorProjOutOfRange(
                                    pos,
                                    *i,
                                    component_types.len(),
                                ))
                            }
                            Some((true, t)) => {
                                g(&t, s, &mut stack_type, &mut verified_ops, component_types)
                            }
                            Some((false, _)) => {
                                return Err(Error::TypeErrorUninitializedRead(pos, *op, *i))
                            }
                        }
                        Ok(())
                    };
                    match mb_tpl {
                        Some(tpl) => match tpl {
                            Type::Tuple(component_types) => {
                                f(component_types, &|t: &Type, s: usize, stack_type: &mut Vec<Type>, verified_ops: &mut Vec<Op2>, component_types: Vec<(bool, Type)>| {
                                    let mut offset = 0;
                                    for i2 in 0..*i {
                                        let (_, t) = &component_types[i2 as usize];
                                        offset += t.size();
                                    }
                                    stack_type.push(t.clone());
                                    verified_ops.push(Op2::Proj(offset, t.size(), s));
                                })?;
                            }
                            Type::Ptr(boxed_t, r) => {
                                if rgn_vars.iter().all(|r2| r.id != r2.id) {
                                    return Err(Error::RegionAccessError(pos, *op, r));
                                }
                                match *boxed_t {
                                    Type::Tuple(component_types) => {
                                        f(component_types, &|t: &Type, _s: usize, stack_type: &mut Vec<Type>, verified_ops: &mut Vec<Op2>, component_types: Vec<(bool, Type)>| {
                                            let mut offset = 0;
                                            for i2 in 0..*i {
                                                let (_, t) = &component_types[i2 as usize];
                                                offset += t.size();
                                            }
                                            stack_type.push(t.clone());
                                            verified_ops.push(Op2::ProjIP(offset, t.size()));
                                        })?;
                                    }
                                    t => return Err(Error::TypeErrorTupleExpected(pos, *op, t)),
                                }
                            }
                            t => return Err(Error::TypeErrorTupleExpected(pos, *op, t)),
                        },
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                }
                Op1::Call => {
                    let mb_type = stack_type.pop();
                    match mb_type {
                        Some(t) => handle_call(pos, &t, &mut stack_type, &mut compile_time_stack)?,
                        None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                    }
                    verified_ops.push(Op2::Call)
                }
                Op1::Print => match stack_type.pop() {
                    Some(Type::I32) => verified_ops.push(Op2::Print),
                    Some(t) => return Err(Error::TypeError(pos, *op, Type::I32, t)),
                    None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                },
                Op1::Lit(lit) => {
                    stack_type.push(Type::I32);
                    verified_ops.push(Op2::Lit(*lit))
                }
                Op1::GlobalFunc(label) => {
                    let t = types
                        .get(label)
                        .ok_or_else(|| panic!("this should be an Err"))?;
                    stack_type.push(t.clone());
                    verified_ops.push(Op2::GlobalFunc(*label))
                }
                Op1::Halt => match stack_type.pop() {
                    Some(Type::I32) => verified_ops.push(Op2::Halt),
                    Some(t) => return Err(Error::TypeError(pos, *op, Type::I32, t)),
                    None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                },
                Op1::Pack => match stack_type.pop() {
                    Some(type_of_hidden) => match compile_time_stack.pop() {
                        Some(CTStackVal::Type(hidden_type)) => match compile_time_stack.pop() {
                            Some(CTStackVal::Type(Type::Exists(
                                id,
                                size_of_hidden,
                                existential_type,
                            ))) => {
                                let unpacked_type = substitute_t(
                                    &existential_type,
                                    &HashMap::from([(id, hidden_type)]),
                                    &HashMap::new(),
                                );
                                if !type_eq(&type_of_hidden, &unpacked_type) {
                                    return Err(Error::TypeError(
                                        pos,
                                        *op,
                                        unpacked_type,
                                        type_of_hidden,
                                    ));
                                }
                                if size_of_hidden != type_of_hidden.size() {
                                    return Err(Error::SizeError(
                                        pos,
                                        *op,
                                        size_of_hidden,
                                        type_of_hidden.size(),
                                    ));
                                }
                                stack_type.push(Type::Exists(id, size_of_hidden, existential_type));
                            }
                            Some(CTStackVal::Type(t)) => {
                                return Err(Error::TypeErrorExistentialExpected(pos, *op, t))
                            }
                            Some(ctval) => {
                                return Err(Error::KindError(pos, *op, Kind::Type, ctval))
                            }
                            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                        },
                        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
                    },
                    None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                },
                Op1::Size(s) => compile_time_stack.push(CTStackVal::Size((*s).try_into().unwrap())),
                Op1::NewRgn => {
                    let id = Id(*label, fresh_id);
                    fresh_id += 1;
                    let r = Region {
                        unique: true,
                        id: id,
                    };
                    rgn_vars.push(r.clone());
                    stack_type.push(Type::Handle(r.clone()));
                    compile_time_stack.push(CTStackVal::Region(r));
                    verified_ops.push(Op2::NewRgn);
                }
                Op1::FreeRgn => match stack_type.pop() {
                    Some(Type::Handle(r)) => match rgn_vars.iter().find(|r2| r.id == r2.id) {
                        Some(r2) if r2.unique => {
                            rgn_vars.retain(|r2| r2.id != r.id);
                            verified_ops.push(Op2::FreeRgn)
                        }
                        Some(_r2) => return Err(Error::UniquenessError(pos, *op, r)),
                        None => return Err(Error::RegionAccessError(pos, *op, r)),
                    },
                    Some(t) => return Err(Error::TypeErrorRegionHandleExpected(pos, *op, t)),
                    None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                },
                Op1::Ptr => handle_ptr(pos, op, &mut compile_time_stack)?,
                Op1::Deref => match stack_type.pop() {
                    Some(Type::Ptr(t, r)) => {
                        if rgn_vars.iter().all(|r2| r.id != r2.id) {
                            return Err(Error::RegionAccessError(pos, *op, r));
                        }
                        let s = t.size();
                        stack_type.push(*t);
                        verified_ops.push(Op2::Deref(s));
                    }
                    Some(t) => return Err(Error::TypeErrorPtrExpected(pos, *op, t)),
                    None => return Err(Error::TypeErrorEmptyStack(pos, *op)),
                },
            },
        }
        pos += 1;
    }
    if quantification_stack.len() > 0 {
        return Err(Error::TypeErrorNonEmptyQuantificationStack(*label));
    }
    // wrap t in the quantifiers from kind_context
    Ok(Stmt2::Func(*label, my_type, verified_ops))
}

fn handle_call(
    pos: u32,
    t: &Type,
    stack_type: &mut Vec<Type>,
    compile_time_stack: &mut Vec<CTStackVal>,
) -> Result<(), Error> {
    match t {
        Type::Func(args) => {
            let arg_ts_needed = args;
            let mut arg_ts_present = vec![];
            for _ in 0..arg_ts_needed.len() {
                match stack_type.pop() {
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
        Type::Forall(var, size, body) => {
            let mb_t = compile_time_stack.pop();
            match mb_t {
                Some(CTStackVal::Type(t)) => {
                    if t.size() != *size {
                        return Err(Error::SizeError(pos, Op1::Call, *size, t.size()));
                    }
                    let new_t = substitute_t(&*body, &HashMap::from([(*var, t)]), &HashMap::new());
                    handle_call(pos, &new_t, stack_type, compile_time_stack)
                }
                Some(ctval) => return Err(Error::KindError(pos, Op1::Call, Kind::Type, ctval)),
                None => return Err(Error::TypeErrorEmptyCTStack(pos, Op1::Call)),
            }
        }
        Type::ForallRegion(var, body, captured_rgns) => {
            let mb_r = compile_time_stack.pop();
            match mb_r {
                Some(CTStackVal::Region(r)) => {
                    if var.unique && captured_rgns.iter().any(|r2| r2.id == r.id) {
                        return Err(Error::RegionAccessError(pos, Op1::Call, r));
                    }
                    let new_t =
                        substitute_t(&*body, &HashMap::new(), &HashMap::from([(var.id, r)]));
                    handle_call(pos, &new_t, stack_type, compile_time_stack)
                }
                Some(ctval) => return Err(Error::KindError(pos, Op1::Call, Kind::Region, ctval)),
                None => return Err(Error::TypeErrorEmptyCTStack(pos, Op1::Call)),
            }
        }
        _ => return Err(Error::TypeErrorFunctionExpected(pos, Op1::Call, t.clone())),
    }
}

fn handle_handle(
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
) -> Result<(), Error> {
    match compile_time_stack.pop() {
        Some(CTStackVal::Region(r)) => {
            compile_time_stack.push(CTStackVal::Type(Type::Handle(r)));
            Ok(())
        }
        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Region, ctval)),
        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
    }
}

fn handle_tuple(
    n: &u8,
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
) -> Result<(), Error> {
    let mut ts = vec![];
    for _ in 0..*n {
        match compile_time_stack.pop() {
            Some(CTStackVal::Type(t)) => ts.push((true, t)),
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        }
    }
    compile_time_stack.push(CTStackVal::Type(Type::Tuple(ts)));
    Ok(())
}

fn handle_some(
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
    fresh_id: &mut u32,
    label: &u32,
    quantification_stack: &mut Vec<Quantification>,
) -> Result<(), Error> {
    match compile_time_stack.pop() {
        Some(CTStackVal::Size(s)) => {
            let id = Id(*label, *fresh_id);
            *fresh_id += 1;
            compile_time_stack.push(CTStackVal::Type(Type::Var(id.clone(), s)));
            quantification_stack.push(Quantification::Exist(id, s));
            Ok(())
        }
        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Size, ctval)),
        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
    }
}

fn handle_all(
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
    fresh_id: &mut u32,
    label: &u32,
    quantification_stack: &mut Vec<Quantification>,
) -> Result<(), Error> {
    match compile_time_stack.pop() {
        Some(CTStackVal::Size(s)) => {
            let id = Id(*label, *fresh_id);
            *fresh_id += 1;
            compile_time_stack.push(CTStackVal::Type(Type::Var(id.clone(), s)));
            quantification_stack.push(Quantification::Forall(id, s));
            Ok(())
        }
        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Size, ctval)),
        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
    }
}

fn handle_rgn(
    next_region_is_unique: &mut bool,
    label: &u32,
    fresh_id: &mut u32,
    compile_time_stack: &mut Vec<CTStackVal>,
    quantification_stack: &mut Vec<Quantification>,
) -> Result<(), Error> {
    let id = Id(*label, *fresh_id);
    let r = Region {
        unique: *next_region_is_unique,
        id: id,
    };
    *fresh_id += 1;
    compile_time_stack.push(CTStackVal::Region(r.clone()));
    quantification_stack.push(Quantification::Region(r));
    Ok(())
}

fn handle_end(
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
    quantification_stack: &mut Vec<Quantification>,
) -> Result<(), Error> {
    match quantification_stack.pop() {
        Some(Quantification::Exist(id, s)) => match compile_time_stack.pop() {
            Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                Some(CTStackVal::Type(Type::Var(id2, _))) if id == id2 => {
                    compile_time_stack.push(CTStackVal::Type(Type::Exists(id, s, Box::new(t))));
                    Ok(())
                }
                Some(CTStackVal::Type(Type::Var(id2, _))) => {
                    return Err(Error::TypeErrorSpecificTypeVarExpected(pos, *op, id, id2))
                }
                Some(CTStackVal::Type(t)) => {
                    return Err(Error::TypeErrorTypeVarExpected(pos, *op, id, t))
                }
                Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
            },
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        },
        Some(Quantification::Forall(id, s)) => match compile_time_stack.pop() {
            Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                Some(CTStackVal::Type(Type::Var(id2, _))) if id == id2 => {
                    compile_time_stack.push(CTStackVal::Type(Type::Forall(id, s, Box::new(t))));
                    Ok(())
                }
                Some(CTStackVal::Type(Type::Var(id2, _))) => {
                    return Err(Error::TypeErrorSpecificTypeVarExpected(pos, *op, id, id2))
                }
                Some(CTStackVal::Type(t)) => {
                    return Err(Error::TypeErrorTypeVarExpected(pos, *op, id, t))
                }
                Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
                None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
            },
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        },
        Some(Quantification::Region(r)) => match compile_time_stack.pop() {
            Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
                Some(CTStackVal::Region(r2)) if r.id == r2.id => {
                    compile_time_stack.push(CTStackVal::Type(Type::ForallRegion(
                        r,
                        Box::new(t),
                        vec![],
                    )));
                    Ok(())
                }
                Some(CTStackVal::Region(r2)) => return Err(Error::RegionError(pos, *op, r, r2)),
                Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Region, ctval)),
                None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
            },
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        },
        None => return Err(Error::TypeErrorEmptyQuantificationStack(pos, *op)),
    }
}

fn handle_func(
    n: &u8,
    pos: u32,
    op: &Op1,
    compile_time_stack: &mut Vec<CTStackVal>,
) -> Result<(), Error> {
    let mut ts = vec![];
    for _ in 0..*n {
        match compile_time_stack.pop() {
            Some(CTStackVal::Type(t)) => ts.push(t),
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        }
    }
    compile_time_stack.push(CTStackVal::Type(Type::Func(ts)));
    Ok(())
}

fn handle_ctget(pos: u32, i: &u8, compile_time_stack: &mut Vec<CTStackVal>) -> Result<(), Error> {
    match compile_time_stack.get(compile_time_stack.len() - 1 - (*i) as usize) {
        Some(ctval) => {
            compile_time_stack.push(ctval.clone());
            Ok(())
        }
        None => {
            return Err(Error::TypeErrorCTGetOutOfRange(
                pos,
                *i,
                compile_time_stack.len(),
            ))
        }
    }
}

fn handle_ptr(pos: u32, op: &Op1, compile_time_stack: &mut Vec<CTStackVal>) -> Result<(), Error> {
    match compile_time_stack.pop() {
        Some(CTStackVal::Type(t)) => match compile_time_stack.pop() {
            Some(CTStackVal::Region(r)) => {
                compile_time_stack.push(CTStackVal::Type(Type::Ptr(Box::new(t), r)));
                Ok(())
            }
            Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Region, ctval)),
            None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
        },
        Some(ctval) => return Err(Error::KindError(pos, *op, Kind::Type, ctval)),
        None => return Err(Error::TypeErrorEmptyCTStack(pos, *op)),
    }
}

/// Perform some variable substitutions within a type.
/// This does not modify the original.
pub fn substitute_t(typ: &Type, tsubs: &HashMap<Id, Type>, rsubs: &HashMap<Id, Region>) -> Type {
    match typ {
        Type::I32 => Type::I32,
        Type::Handle(r) => Type::Handle(substitute_r(r, rsubs)),
        Type::Tuple(ts) => Type::Tuple(
            ts.iter()
                .map(|(init, t)| (*init, substitute_t(t, tsubs, rsubs)))
                .collect(),
        ),
        Type::Ptr(t, r) => Type::Ptr(
            Box::new(substitute_t(t, tsubs, rsubs)),
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
        Type::ForallRegion(id, t, captured_rgns) => {
            let mut captured_rgns = captured_rgns.clone();
            for (_, r) in rsubs {
                if r.unique {
                    captured_rgns.push(*r);
                }
            }
            Type::ForallRegion(*id, Box::new(substitute_t(t, tsubs, rsubs)), captured_rgns)
        }
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
        (Type::Tuple(ts1), Type::Tuple(ts2)) => {
            ts1.len() == ts2.len() && {
                let mut ts2 = ts2.iter();
                for (init1, t1) in ts1 {
                    let (init2, t2) = ts2.next().unwrap();
                    if init1 != init2 || !type_eq(t1, t2) {
                        return false;
                    }
                }
                return true;
            }
        }
        (Type::Ptr(t1, r1), Type::Ptr(t2, r2)) => r1 == r2 && type_eq(t1, t2),
        (Type::Var(id1, repr1), Type::Var(id2, repr2)) => id1 == id2 && repr1 == repr2,
        (Type::Func(ts1), Type::Func(ts2)) => {
            ts1.iter().zip(ts2.iter()).all(|(t1, t2)| type_eq(&t1, &t2))
        }
        (Type::Exists(id1, repr1, t1), Type::Exists(id2, repr2, t2)) => {
            let mut sub = HashMap::new();
            sub.insert(*id2, Type::Var(*id1, repr1.clone()));
            let t2_subbed = substitute_t(t2, &sub, &HashMap::new());
            repr1 == repr2 && type_eq(t1, &t2_subbed)
        }
        (Type::Forall(id1, size1, body1), Type::Forall(id2, size2, body2)) => {
            let mut sub = HashMap::new();
            sub.insert(*id2, Type::Var(*id1, size1.clone()));
            let body2_subbed = substitute_t(&body2, &sub, &HashMap::new());
            size1 == size2 && type_eq(body1, &body2_subbed)
        }
        (Type::ForallRegion(r1, body1, _captured_rgns1), Type::ForallRegion(r2, body2, _captured_rgns2)) => {
            let mut sub = HashMap::new();
            sub.insert(r2.id, *r1);
            let body2_subbed = substitute_t(&body2, &HashMap::new(), &sub);
            type_eq(body1, &body2_subbed)
        }
        (_, _) => false,
    }
}

fn setup_verifier(t: &Type) -> Result<(Vec<CTStackVal>, Vec<Type>), Error> {
    match t {
        Type::Forall(id, s, t) => {
            let (mut ct_stack, param_types) = setup_verifier(t)?;
            ct_stack.push(CTStackVal::Type(Type::Var(*id, *s)));
            Ok((ct_stack, param_types))
        }
        Type::ForallRegion(r, t, _captured_rgns) => {
            let (mut ct_stack, param_types) = setup_verifier(t)?;
            ct_stack.push(CTStackVal::Region(*r));
            Ok((ct_stack, param_types))
        }
        Type::Func(param_ts) => {
            let mut param_ts = param_ts.to_vec();
            param_ts.reverse();
            Ok((vec![], param_ts.into()))
        }
        _ => panic!("this should be an Err"),
    }
}
