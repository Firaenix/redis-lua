use proc_macro_hack::proc_macro_hack;

mod script;

#[proc_macro_hack]
pub use redis_lua_derive::{lua, lua_str};
pub use script::Script;