use std::fmt;

pub mod visit;
use crate::visit::Walkable;

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Walkable)]
pub struct Label {
    pub ident: Ident,
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "label({:?})", self.ident)
    }
}

#[derive(Clone, Debug, Walkable)]
pub struct Path {
    pub segments: Vec<PathSegment>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub struct PathSegment {
    pub ident: Ident,
    pub args: Option<GenericArgs>,
}

#[derive(Clone, Debug, Walkable)]
pub struct GenericArgs {
    pub args: Vec<GenericArg>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum GenericArg {
    Arg(Ty),
    Constraint(AssocItemConstraint),
}

#[derive(Clone, Debug, Walkable)]
pub struct AssocItemConstraint {
    pub ident: Ident,
    pub gen_args: Option<GenericArgs>,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
#[walkable(hookable)]
pub struct Ty {
    pub kind: TyKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum TyKind {
    Array(Box<Ty>),
    Never,
    Tup(Vec<Box<Ty>>),
    Path(Path),
    Paren(Box<Ty>),
    Fn(Box<FnTy>),
    Infer,
    ImplicitSelf,
    Err,
}

#[derive(Clone, Debug, Walkable)]
pub struct Generics {
    pub params: Vec<GenericParam>,
    pub where_clause: WhereClause,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub struct WhereClause {
    pub predicates: Vec<WherePredicate>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub struct GenericParam {
    pub ident: Ident,
    pub bounds: GenericBounds,
    pub default: Option<Ty>,
    pub annotations: AnnotationVec,
}

pub type GenericBounds = Vec<Ty>;

#[derive(Clone, Debug, Walkable)]
pub struct WherePredicate {
    pub bounded_ty: Box<Ty>,
    pub bounds: GenericBounds,
}

#[derive(Clone, Debug, Walkable)]
pub struct Lit {
    pub kind: LitKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Walkable)]
pub enum LitKind {
    Str(String),
    Char(char),
    Int(u128),
    Float(String),
    Bool(bool),
}

#[derive(Clone, Debug, Walkable)]
pub struct MetaItem {
    pub path: Path,
    pub kind: MetaItemKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum MetaItemKind {
    Word,
    List(Vec<MetaItemInner>),
    NameValue(MetaItemLit),
}

#[derive(Clone, Debug, Walkable)]
pub enum MetaItemInner {
    MetaItem(MetaItem),
    Lit(MetaItemLit),
}

pub type MetaItemLit = Lit;

#[derive(Clone, Debug, Walkable)]
pub struct Annotation {
    pub item: MetaItem,
    pub style: AnnotationStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Walkable)]
pub enum AnnotationStyle {
    Outer,
    Inner,
}

pub type AnnotationVec = Vec<Annotation>;

#[derive(Clone, Debug, Walkable)]
#[walkable(hookable)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
#[walkable(hookable)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum StmtKind {
    Let(Box<Local>),
    Item(Box<Item>),
    Expr(Box<Expr>),
    Semi(Box<Expr>),
    Empty,
}

#[derive(Clone, Debug, Walkable)]
pub struct Local {
    pub pat: Box<Pat>,
    pub ty: Option<Box<Ty>>,
    pub kind: LocalKind,
    pub span: Span,
    pub annotations: AnnotationVec,
}

#[derive(Clone, Debug, Walkable)]
pub enum LocalKind {
    Decl,
    Init(Box<Expr>),
    InitElse(Box<Expr>, Box<Block>),
}

#[derive(Clone, Debug, Walkable)]
#[walkable(hookable)]
pub struct Expr {
    pub annotations: AnnotationVec,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum ExprKind {
    Array(Vec<Box<Expr>>),
    Call(Box<Expr>, Vec<Box<Expr>>),
    MethodCall(Box<MethodCall>),
    Tup(Vec<Box<Expr>>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Lit(Lit),
    Let(Box<Pat>, Box<Expr>, Span),
    If(Box<Expr>, Box<Block>, Option<Box<Expr>>),
    While(Box<Expr>, Box<Block>, Option<Label>),
    ForLoop {
        pat: Box<Pat>,
        iter: Box<Expr>,
        body: Box<Block>,
        label: Option<Label>,
    },
    Loop(Box<Block>, Option<Label>, Span),
    Match(Box<Expr>, Vec<Arm>),
    Closure(Box<Closure>),
    Block(Box<Block>, Option<Label>),
    Assign(Box<Expr>, Box<Expr>, Span),
    AssignOp(AssignOp, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, Ident),
    Index(Box<Expr>, Box<Expr>, Span),
    Range(Option<Box<Expr>>, Option<Box<Expr>>, RangeLimits),
    Underscore,
    Path(Option<Box<QSelf>>, Path),
    Break(Option<Label>, Option<Box<Expr>>),
    Continue(Option<Label>),
    Ret(Option<Box<Expr>>),
    Struct(Box<StructExpr>),
    Paren(Box<Expr>),
    Try(Box<Expr>),
    Cast(Box<Expr>, Box<Ty>),
    Err,
}

#[derive(Clone, Debug, Walkable)]
pub struct StructExpr {
    pub qself: Option<Box<QSelf>>,
    pub path: Path,
    pub fields: Vec<ExprField>,
    pub rest: Option<Box<Expr>>,
}

#[derive(Clone, Debug, Walkable)]
pub struct ExprField {
    pub annotations: AnnotationVec,
    pub span: Span,
    pub ident: Ident,
    pub expr: Box<Expr>,
}

#[derive(Clone, Debug, Walkable)]
pub struct QSelf {
    pub ty: Box<Ty>,
    pub trait_path: Option<Path>,
}

#[derive(Clone, Debug, Walkable)]
pub enum RangeLimits {
    HalfOpen,
    Closed,
}

#[derive(Clone, Debug, Walkable)]
pub struct Arm {
    pub annotations: AnnotationVec,
    pub pat: Box<Pat>,
    pub guard: Option<Box<Guard>>,
    pub body: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub struct Guard {
    pub cond: Expr,
}

#[derive(Clone, Debug, Walkable)]
pub struct Closure {
    pub fn_decl: Box<FnDecl>,
    pub body: Box<Expr>,
}

#[derive(Clone, Debug, Walkable)]
pub struct FnDecl {
    pub inputs: Vec<Param>,
    pub output: FnRetTy,
}

#[derive(Clone, Debug, Walkable)]
pub enum FnRetTy {
    Default(Span),
    Ty(Box<Ty>),
}

#[derive(Clone, Debug, Walkable)]
pub struct FnTy {
    pub inputs: Vec<Box<Ty>>,
    pub output: FnRetTy,
}

#[derive(Clone, Debug, Walkable)]
pub struct Param {
    pub annotations: AnnotationVec,
    pub ty: Option<Box<Ty>>,
    pub pat: Box<Pat>,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub struct MethodCall {
    pub seg: PathSegment,
    pub receiver: Box<Expr>,
    pub args: Vec<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Item<K = ItemKind> {
    pub span: Span,
    pub vis: Visibility,
    pub annotations: AnnotationVec,
    pub kind: K,
}

#[derive(Clone, Debug, Walkable)]
pub struct Visibility {
    pub kind: VisibilityKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum VisibilityKind {
    Public,
    Restricted { path: Box<Path> },
    Inherited,
}

#[derive(Clone, Debug, Walkable)]
pub enum ItemKind {
    Use(UseTree),
    Fn(Box<Fn>),
    Mod(Ident, ModKind),
    TyAlias(Box<TyAlias>),
    Enum(Ident, Generics, EnumDef),
    Struct(Ident, Generics, VariantData),
    Trait(Box<Trait>),
    Impl(Impl),
}

#[derive(Clone, Debug, Walkable)]
pub struct Trait {
    pub ident: Ident,
    pub generics: Generics,
    pub bounds: GenericBounds,
    pub items: Vec<Box<AssocItem>>,
}

#[derive(Clone, Debug, Walkable)]
pub struct EnumDef {
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, Walkable)]
pub struct Variant {
    pub annotations: AnnotationVec,
    pub span: Span,
    pub vis: Visibility,
    pub ident: Ident,
    pub data: VariantData,
}

#[derive(Clone, Debug, Walkable)]
pub enum VariantData {
    Struct(Vec<FieldDef>),
    Tuple(Vec<FieldDef>),
    Unit,
}

#[derive(Clone, Debug, Walkable)]
pub struct FieldDef {
    pub annotations: AnnotationVec,
    pub span: Span,
    pub vis: Visibility,
    pub ident: Option<Ident>,
    pub ty: Option<Box<Ty>>,
}

#[derive(Clone, Debug, Walkable)]
pub struct Impl {
    pub generics: Generics,
    pub of_trait: Option<Box<Path>>,
    pub self_ty: Box<Ty>,
    pub items: Vec<Box<AssocItem>>,
}

pub type AssocItem = Item<AssocItemKind>;

#[derive(Clone, Debug, Walkable)]
pub enum AssocItemKind {
    Fn(Box<Fn>),
    Type(Box<TyAlias>),
}

#[derive(Clone, Debug, Walkable)]
pub struct TyAlias {
    pub ident: Ident,
    pub generics: Generics,
    pub after_where_clause: WhereClause,
    pub bounds: GenericBounds,
    pub ty: Option<Box<Ty>>,
}

#[derive(Clone, Debug, Walkable)]
pub enum ModKind {
    Loaded(Vec<Box<Item>>),
    Unloaded,
}

#[derive(Clone, Debug, Walkable)]
pub struct Fn {
    pub ident: Ident,
    pub generics: Generics,
    pub sig: FnDecl,
    pub body: Option<Box<Block>>,
}

#[derive(Clone, Debug, Walkable)]
pub struct UseTree {
    pub prefix: Path,
    pub kind: UseTreeKind,
}

#[derive(Clone, Debug, Walkable)]
pub enum UseTreeKind {
    Simple(Option<Ident>),
    Nested { items: Vec<UseTree>, span: Span },
    Glob(Span),
}

#[derive(Clone, Debug, Walkable)]
#[walkable(hookable)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Clone, Debug, Walkable)]
pub enum PatKind {
    Missing,
    Wild,
    Ident(Ident, Option<Box<Pat>>),
    Struct(Option<Box<QSelf>>, Path, Vec<PatField>, PatFieldsRest),
    TupleStruct(Option<Box<QSelf>>, Path, Vec<Pat>),
    Or(Vec<Pat>),
    Path(Option<Box<QSelf>>, Path),
    Tuple(Vec<Pat>),
    Expr(Box<Expr>),
    Range(Option<Box<Expr>>, Option<Box<Expr>>, RangeEnd, Span),
    Array(Vec<Pat>),
    Rest,
    Never,
    Paren(Box<Pat>),
    Err,
}

type PatFieldsRest = Option<Span>;

#[derive(Clone, Debug, Walkable)]
pub struct PatField {
    pub ident: Ident,
    pub pat: Box<Pat>,
    pub annotations: AnnotationVec,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Walkable)]
pub enum RangeEnd {
    Included,
    Excluded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fixity {
    Left,
    Right,
    None,
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub enum ExprPrecedence {
    Jump,
    Assign,
    Range,
    LOr,
    LAnd,
    Compare,
    BitOr,
    BitXor,
    BitAnd,
    Shift,
    Sum,
    Product,
    Cast,
    Prefix,
    Unambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Walkable)]
pub enum AssignOpKind {
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    RemAssign,
    BitXorAssign,
    BitAndAssign,
    BitOrAssign,
    ShlAssign,
    ShrAssign,
}

impl AssignOpKind {
    pub fn as_str(&self) -> &'static str {
        use AssignOpKind::*;
        match self {
            AddAssign => "+=",
            SubAssign => "-=",
            MulAssign => "*=",
            DivAssign => "/=",
            RemAssign => "%=",
            BitXorAssign => "^=",
            BitAndAssign => "&=",
            BitOrAssign => "|=",
            ShlAssign => "<<=",
            ShrAssign => ">>=",
        }
    }

    pub fn is_by_value(self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Walkable)]
pub struct AssignOp {
    pub kind: AssignOpKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Walkable)]
pub enum UnOp {
    Not,
    Neg,
}

impl UnOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnOp::Not => "!",
            UnOp::Neg => "-",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Walkable)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    BitXor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
}

impl BinOpKind {
    pub fn as_str(&self) -> &'static str {
        use BinOpKind::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            And => "&&",
            Or => "||",
            BitXor => "^",
            BitAnd => "&",
            BitOr => "|",
            Shl => "<<",
            Shr => ">>",
            Eq => "==",
            Lt => "<",
            Le => "<=",
            Ne => "!=",
            Ge => ">=",
            Gt => ">",
        }
    }

    pub fn is_lazy(&self) -> bool {
        matches!(self, BinOpKind::And | BinOpKind::Or)
    }

    pub fn precedence(&self) -> ExprPrecedence {
        use BinOpKind::*;
        match *self {
            Mul | Div | Rem => ExprPrecedence::Product,
            Add | Sub => ExprPrecedence::Sum,
            Shl | Shr => ExprPrecedence::Shift,
            BitAnd => ExprPrecedence::BitAnd,
            BitXor => ExprPrecedence::BitXor,
            BitOr => ExprPrecedence::BitOr,
            Lt | Gt | Le | Ge | Eq | Ne => ExprPrecedence::Compare,
            And => ExprPrecedence::LAnd,
            Or => ExprPrecedence::LOr,
        }
    }

    pub fn fixity(&self) -> Fixity {
        use BinOpKind::*;
        match self {
            Eq | Ne | Lt | Le | Gt | Ge => Fixity::None,
            Add | Sub | Mul | Div | Rem | And | Or | BitXor | BitAnd | BitOr | Shl | Shr => {
                Fixity::Left
            }
        }
    }

    pub fn is_comparison(self) -> bool {
        use BinOpKind::*;
        match self {
            Eq | Ne | Lt | Le | Gt | Ge => true,
            Add | Sub | Mul | Div | Rem | And | Or | BitXor | BitAnd | BitOr | Shl | Shr => false,
        }
    }

    pub fn is_by_value(self) -> bool {
        !self.is_comparison()
    }
}

#[derive(Clone, Debug, Walkable)]
pub struct BinOp {
    pub kind: BinOpKind,
    pub span: Span,
}
