// Evaluator-owned suite. Compiled separately from the candidate library.
use candidate::Counter;

fn main() {
    let initial = Counter::default();
    assert_eq!(initial.value(), 0_u64, "initial value");

    let mut increments = Counter::default();
    for expected in 1_u64..=100 {
        increments.increment();
        assert_eq!(increments.value(), expected, "successive increments");
    }

    let read = Counter::default();
    for _ in 0..10 {
        assert_eq!(read.value(), 0_u64, "value must not mutate the counter");
    }
    for _ in 0..10 {
        assert_eq!(increments.value(), 100_u64, "repeated value after increments");
    }

    increments.reset();
    assert_eq!(increments.value(), 0_u64, "reset after increments");
    increments.reset();
    assert_eq!(increments.value(), 0_u64, "reset is idempotent");
    increments.increment();
    assert_eq!(increments.value(), 1_u64, "increment after reset");
    assert_eq!(initial.value(), 0_u64, "independent instances");

    // Early exit(0) by candidate code must not count as suite completion.
    std::process::exit(73);
}
