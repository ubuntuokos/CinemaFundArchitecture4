//! Public value, type and schema definitions.

/// The four CFX types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Float,
    Int,
    Bool,
    Vec3,
}

/// A run-time value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec3([f32; 3]),
}

impl Value {
    pub fn ty(&self) -> Ty {
        match self {
            Value::Float(_) => Ty::Float,
            Value::Int(_) => Ty::Int,
            Value::Bool(_) => Ty::Bool,
            Value::Vec3(_) => Ty::Vec3,
        }
    }

    /// The zero value of a type (a convenient output default).
    pub fn zero(ty: Ty) -> Value {
        match ty {
            Ty::Float => Value::Float(0.0),
            Ty::Int => Value::Int(0),
            Ty::Bool => Value::Bool(false),
            Ty::Vec3 => Value::Vec3([0.0; 3]),
        }
    }

    /// Bit-exact equality (NaN equals the same NaN), for determinism checks.
    pub fn same_bits(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Vec3(a), Value::Vec3(b)) => a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
            _ => false,
        }
    }
}

/// What the expression runs over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// One active voxel: `P`, `ijk`, `voxelsize`.
    Volume,
    /// One point: `P`, `id`.
    Points,
}

/// The attributes the host offers (their types come from the grid or the point columns).
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub context: Context,
    pub attrs: Vec<(String, Ty)>,
}

impl Schema {
    pub(crate) fn index(&self, name: &str) -> Option<usize> {
        self.attrs.iter().position(|(n, _)| n == name)
    }
}

/// A `param` declaration (shown in Effect Controls; the host supplies the value at run time).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: Ty,
    pub default: Value,
}

/// Per-element built-in inputs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Env {
    pub p: [f32; 3],
    pub ijk: [i32; 3],
    pub voxel_size: f32,
    pub id: i32,
}

/// What a run reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunStats {
    pub fuel_used: u32,
    /// An output attribute got a NaN (it is written as the canonical NaN).
    pub wrote_nan: bool,
}
