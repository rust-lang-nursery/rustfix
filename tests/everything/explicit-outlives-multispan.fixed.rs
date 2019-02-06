#![allow(dead_code)]
#![deny(explicit_outlives_requirements)]

use std::fmt::Debug;

struct TeeOutlivesAyYooBeeIsDebug<'a, 'b, T, U: Debug> {
    tee: &'a T,
    yoo: &'b U
}

fn main() {}
