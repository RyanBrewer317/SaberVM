#[derive(Clone, Copy, Debug)]
pub enum OpCode1 {
    Op1Req(),     // 0x00
    Op1Region(),  // 0x01
    Op1Heap(),    // 0x02
    Op1Cap(),     // 0x03
    Op1CapLE(),   // 0x04
    Op1Own(),     // 0x05
    Op1Read(),    // 0x06
    Op1Both(),    // 0x07
    Op1Handle(),  // 0x08
    Op1i32(),     // 0x09
    Op1End(),     // 0x0A
    Op1Mut(),     // 0x0B
    Op1Tuple(u8), // 0x0C
    Op1Arr(),     // 0x0D
    Op1All(),     // 0x0E
    Op1Some(),    // 0x0F
    Op1Emos(),    // 0x10
    Op1Func(u8),  // 0x11
    Op1CTGet(u8), // 0x12
    Op1CTPop(),   // 0x13
    Op1Unpack(),  // 0x14
    Op1Get(u8),   // 0x15
    Op1Init(u8),  // 0x16
    Op1Malloc(),  // 0x17
    Op1Proj(u8),  // 0x18
    Op1Clean(u8), // 0x19
    Op1Call(),    // 0x1A
}

#[derive(Clone, Copy, Debug)]
pub enum OpCode2 {
    Op2Get(u8),
    Op2Init(u8),
    Op2Malloc(u8),
    Op2Proj(u8),
    Op2Clean(u8, u8),
    Op2Call(),
}

#[derive(Debug)]
pub enum Stmt1 {
    Func1(i32, Vec<OpCode1>),
}

#[derive(Debug)]
pub enum Stmt2 {
    Func2(i32, Type, Vec<OpCode2>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    KRegion,
    KType,
    KCapability(Option<CapabilityRef>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id(pub i32, pub i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    RegionVar(Id),
    Heap(),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Owned(Region),
    NotOwned(Region),
    CapVar(Id),
    CapVarBounded(Id, CapabilityRef),
}

pub struct CapabilityPool(pub Vec<Vec<Capability>>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityRef(u32);
impl CapabilityPool {
    pub fn get(&self, r: CapabilityRef) -> &Vec<Capability> {
        let CapabilityPool(v) = self;
        let CapabilityRef(i) = r;
        &v[i as usize]
    }
    pub fn add(&mut self, cap: Vec<Capability>) -> CapabilityRef {
        let CapabilityPool(v) = self;
        let idx = v.len();
        v.push(cap);
        CapabilityRef(idx.try_into().expect("too many capabilities in the pool"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Ti32(),
    THandle(Region),
    TMutable(TypeRef),
    TTuple(TypeListRef, Region),
    TArray(TypeRef),
    TVar(Id),
    TForall(Id, Kind, TypeRef),
    TExists(Id, TypeRef),
    TFunc(CapabilityRef, TypeListRef),
    TGuess(i32),
}

pub struct TypePool(pub Vec<Type>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeRef(u32);
impl TypePool {
    pub fn get(&self, r: TypeRef) -> &Type {
        let TypePool(v) = self;
        let TypeRef(i) = r;
        &v[i as usize]
    }
    pub fn add(&mut self, t: Type) -> TypeRef {
        let TypePool(v) = self;
        let idx = v.len();
        v.push(t);
        TypeRef(idx.try_into().expect("too many types in the pool"))
    }
}

pub struct TypeListPool(pub Vec<Vec<TypeRef>>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeListRef(u32);
impl TypeListPool {
    pub fn get(&self, r: TypeListRef) -> &Vec<TypeRef> {
        let TypeListPool(v) = self;
        let TypeListRef(i) = r;
        &v[i as usize]
    }
    pub fn add(&mut self, ts: Vec<TypeRef>) -> TypeListRef {
        let TypeListPool(v) = self;
        let idx = v.len();
        v.push(ts);
        TypeListRef(idx.try_into().expect("too many type lists in the pool"))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CTStackVal {
    CTRegion(Region),
    CTCapability(CapabilityRef),
    CTType(TypeRef),
}

pub fn get_kind_str(ctval: CTStackVal) -> String {
    match ctval {
        CTStackVal::CTCapability(_) => "capability".to_owned(),
        CTStackVal::CTRegion(_) => "region".to_owned(),
        CTStackVal::CTType(_) => "type".to_owned(),
    }
}

pub fn get_op_str(byte: u8) -> String {
    (match byte {
        0x00 => "req",
        0x01 => "region",
        0x02 => "heap",
        0x03 => "cap",
        0x04 => "cap_le",
        0x05 => "own",
        0x06 => "read",
        0x07 => "both",
        0x08 => "handle",
        0x09 => "i32",
        0x0A => "END_FUNC",
        0x0B => "mut",
        0x0C => "tuple",
        0x0D => "arr",
        0x0E => "all",
        0x0F => "some",
        0x10 => "emos",
        0x11 => "func",
        0x12 => "ct_get",
        0x13 => "ct_pop",
        0x14 => "unpack",
        0x15 => "get",
        0x16 => "init",
        0x17 => "malloc",
        0x18 => "proj",
        0x19 => "clean",
        0x20 => "call",
        _ => panic!("unknown opcode {}", byte),
    })
    .to_owned()
}

pub fn pretty_kind(k: Kind) -> String {
    (match k {
        Kind::KCapability(_) => "capability",
        Kind::KRegion => "region",
        Kind::KType => "type",
    })
    .to_owned()
}

#[derive(Debug)]
pub enum Error {
    SyntaxErrorParamNeeded(u8),
    SyntaxErrorUnknownOp(u8),
    TypeErrorEmptyCTStack(OpCode1),
    KindErrorReq(CTStackVal),
    KindError(OpCode1, Kind, CTStackVal),
    TypeErrorEmptyExistStack(OpCode1),
    TypeErrorParamOutOfRange(OpCode1),
    TypeErrorExistentialExpected(TypeRef),
    TypeErrorEmptyStack(OpCode1),
    CapabilityError(OpCode1, CapabilityRef),
    TypeErrorInit(TypeRef, TypeRef),
    TypeErrorTupleExpected(OpCode1, TypeRef),
    TypeErrorRegionHandleExpected(OpCode1, TypeRef),
    TypeErrorFunctionExpected(OpCode1, TypeRef),
    TypeErrorNonEmptyExistStack(),
}
