//! Step logging: `step!` runs an expression that returns a `Result`, prints
//! `[ OK ]` with the value or `[FAIL]` with the error, and passes the result on.

pub const OK: &str = "\x1b[32m[ OK ]\x1b[0m";
pub const FAIL: &str = "\x1b[31m[FAIL]\x1b[0m";

/// How a step's successful value is printed. Addresses and handles print as hex.
pub trait Show {
    fn show(&self) -> String;
}

impl Show for u64 {
    fn show(&self) -> String {
        format!("{self:#x}")
    }
}

impl Show for u32 {
    fn show(&self) -> String {
        format!("{self:#x}")
    }
}

impl Show for u8 {
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Show for i32 {
    fn show(&self) -> String {
        self.to_string()
    }
}

impl Show for String {
    fn show(&self) -> String {
        format!("{self:?}")
    }
}

/// `step!(enabled, expr, "description {}", args...)`
#[macro_export]
macro_rules! step {
    ($on:expr, $e:expr, $($fmt:tt)+) => {{
        let r = $e;
        if $on {
            match &r {
                Ok(v) => println!("{} {} -> {}", $crate::log::OK, format_args!($($fmt)+), $crate::log::Show::show(v)),
                Err(e) => println!("{} {}: {:#}", $crate::log::FAIL, format_args!($($fmt)+), e),
            }
        }
        r
    }};
}
