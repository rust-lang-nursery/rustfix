#![allow(dead_code)]
#![deny(explicit_outlives_requirements)]

use std::fmt::Debug;

struct TeeOutlivesAyYooBeeIsDebug<'a, 'b, T: 'a, U: 'b + Debug> {
    tee: &'a T,
    yoo: &'b U
}

fn main() {}
