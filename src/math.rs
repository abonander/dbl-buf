//! Math functions useful to this crate

/// Get the Least Common Multiple of `x` and `y`
pub fn lcm(x: usize, y: usize) -> usize {
    x * y / gcf(x, y)
}

/// Get the Greatest Common Factor(/Divisor) of `x` and `y`
pub fn gcf(x: usize, y: usize) -> usize {
    let (mut num, mut denom) = if x > y { (x, y) } else { (y, x) };

    let mut rem;

    while {rem = num % denom; rem != 0} {
        num = denom;
        denom = rem;
    }

    denom
}

/// Get the next multiple of `base` up from `from`.
pub fn next_multiple(from: usize, base: usize) -> usize {
    if base.is_power_of_two() {
        (from + base) & !(base - 1)
    } else {   
        (from / base + if from % base != 0 { 1 } else { 0 })
            * base
    }
} 
