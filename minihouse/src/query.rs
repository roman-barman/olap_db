mod execute;

use crate::aggregate::AggKind;
pub use crate::query::execute::execute;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum CmpOp {
    Gt,
    Lt,
    Eq,
}

#[derive(Debug, Clone)]
pub struct SimpleQuery<'a> {
    pub filter: Option<(&'a str, CmpOp, Value)>,
    pub aggregate: (&'a str, AggKind),
}
