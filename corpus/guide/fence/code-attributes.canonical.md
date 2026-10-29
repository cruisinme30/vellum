```rust {title="limits.rs" lines="1" hl="3-4"}
pub fn allowed(rate: u32, burst: u32) -> bool {
    let budget = rate.saturating_mul(2);
    let used = burst;
    used <= budget
}
```
