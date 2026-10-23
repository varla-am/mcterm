//! Everything mcterm prints starts with "mcterm: ", so it stands out from the game's own output.
macro_rules! say {
    ($($arg:tt)*) => { println!("mcterm: {}", format_args!($($arg)*)) };
}
