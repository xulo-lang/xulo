//! Service (`spawn.process`) runtime regression tests.
//!
//! A service is an isolated object plus a message queue (spec
//! `concurrency.md`): `send` enqueues a deep copy of the message and returns a
//! `Task` that settles with `reply`'s operand; `on_message` runs to completion
//! (through every `await`) before the next message; `stop` cancels everything
//! still queued.

use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

fn run(src: &str) -> Vec<String> {
    let interp = Interpreter::new();
    let program = parse_program(src).unwrap();
    interp.run(&program).unwrap()
}

/// The spec services example: a Tally service accumulates and replies with
/// the running total.
#[test]
fn service_reply_settles_send_task() {
    let out = run(r#"
        struct Tally { total: number }
        impl Tally {
            fn on_message(mut self, cmd: number) {
                self.total = self.total + cmd
                reply(self.total)
            }
        }
        async fn main() {
            let worker = spawn.process Tally { total: 0 }
            let sum = await worker.send(41)
            print(sum)
            worker.stop()
        }
        "#);
    assert_eq!(out, vec!["41"]);
}

/// Messages are handled one at a time, in arrival order: `on_message` for the
/// first send completes before the second begins, so the printed sequence is
/// the running total in order.
#[test]
fn service_handles_messages_in_fifo_order() {
    let out = run(r#"
        struct Counter { n: number }
        impl Counter {
            fn on_message(mut self, v: number) {
                self.n = self.n + v
                reply(self.n)
            }
        }
        async fn main() {
            let svc = spawn.process Counter { n: 0 }
            let a = await svc.send(1)
            print(a)
            let b = await svc.send(2)
            print(b)
            let c = await svc.send(3)
            print(c)
            svc.stop()
        }
        "#);
    assert_eq!(out, vec!["1", "3", "6"]);
}

/// An async `on_message` holds the service busy until it settles: the next
/// message waits for every `await` inside the handler to finish.
#[test]
fn async_on_message_defers_next_message() {
    let out = run(r#"
        struct Box { v: number }
        async fn slow(x: number): number { x }
        impl Box {
            async fn on_message(mut self, x: number) {
                self.v = await slow(x)
                reply(self.v)
            }
        }
        async fn main() {
            let svc = spawn.process Box { v: 0 }
            let a = await svc.send(7)
            print(a)
            let b = await svc.send(9)
            print(b)
            svc.stop()
        }
        "#);
    assert_eq!(out, vec!["7", "9"]);
}

/// `stop` cancels messages still queued: they never reach `on_message` (the
/// async handler is still busy with the first message when they are enqueued
/// and cancelled). The in-flight message still settles with its reply.
#[test]
fn stop_cancels_queued_messages() {
    let out = run(r#"
        async fn slow(n: number): number { n }
        struct S { n: number }
        impl S {
            async fn on_message(mut self, v: number) {
                self.n = self.n + v
                let _unused = await slow(0)
                reply(self.n)
            }
        }
        async fn main() {
            let svc = spawn.process S { n: 0 }
            let inflight = svc.send(1)
            let queued = svc.send(2)
            let later = svc.send(3)
            svc.stop()
            let r = await inflight
            print(r)
        }
        "#);
    // 2 and 3 were cancelled by stop before their handlers ran, so the only
    // reply is the first message's.
    assert_eq!(out, vec!["1"]);
}

/// A service that never calls `reply` settles the send with null (spec: no
/// reply operand means the task settles with unit).
#[test]
fn on_message_without_reply_settles_null() {
    let out = run(r#"
        struct Nop { seen: number }
        impl Nop {
            fn on_message(mut self, v: number) {
                self.seen = v
            }
        }
        async fn main() {
            let svc = spawn.process Nop { seen: 0 }
            let r = await svc.send(5)
            print(str(r))
            svc.stop()
        }
        "#);
    assert_eq!(out, vec!["null"]);
}

/// The service's state is isolated from the spawner's bindings: mutating the
/// value the spawn copied from does not touch the service, and vice versa.
#[test]
fn service_state_is_isolated_from_spawner() {
    let out = run(r#"
        struct Acc { v: number }
        impl Acc {
            fn on_message(mut self, delta: number) {
                self.v = self.v + delta
                reply(self.v)
            }
        }
        async fn main() {
            let mut start = 10
            let svc = spawn.process Acc { v: start }
            start = 99
            let got = await svc.send(1)
            print(got)
            print(start)
            svc.stop()
        }
        "#);
    // The service saw the copied-in value 10, not the spawner's later 99.
    assert_eq!(out, vec!["11", "99"]);
}

/// `Task::resolve` yields the value immediately; `Task::all` collects results in
/// argument order; `Task::race` takes whichever settles first.
#[test]
fn task_namespace_combinators_settle() {
    let out = run(r#"
        async fn work(n: number): number { n }
        async fn main() {
            let ready = Task::resolve(42)
            print(await ready)
            let a = work(1)
            let b = work(2)
            print(await Task::all([a, b]))
            let c = work(3)
            let d = work(4)
            print(await Task::race([c, d]))
        }
        "#);
    assert_eq!(out, vec!["42", "[1, 2]", "3"]);
}

/// A rejection propagates through `Task::all`: the combined task settles
/// rejected and awaiting it fails the run.
#[test]
fn task_all_propagates_rejection() {
    let interp = Interpreter::new();
    let program = parse_program(
        r#"
        async fn boom(): number {
            panic("nope")
        }
        async fn main() {
            let t = boom()
            print(await Task::all([t]))
        }
        "#,
    )
    .unwrap();
    let result = interp.run(&program);
    assert!(result.is_err(), "Task::all of a rejected task must fail");
}

/// `Task::timeout` wraps a task that settles in time: the combined task
/// resolves with `Result::Ok(v)`, and the pending deadline must not stall the
/// loop — the run finishes far inside its 10-second budget.
#[test]
fn task_timeout_wraps_a_task_that_settles_in_time() {
    let start = std::time::Instant::now();
    let out = run(r#"
        async fn main() {
            let r = await Task::timeout(Task::resolve(41), 10000)
            match r {
                Result::Ok(v) => print(v)
                Result::Err(e) => print(e.message)
            }
        }
        "#);
    assert_eq!(out, vec!["41"]);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "a task that settled early must not wait out its deadline"
    );
}

/// Past the deadline the combined task settles with
/// `Result::Err(Error("timed out"))`. The wrapped task here is a service send
/// whose `on_message` deadlocks on a self-send (the service is already busy
/// handling it, so the nested message never runs) and therefore never settles:
/// only the timer can complete the await, and the late rejection of the
/// wrapped task after `stop` finds the combined task already settled.
#[test]
fn task_timeout_expires_on_a_task_that_never_settles() {
    let out = run(r#"
        struct Relay { n: number }
        impl Relay {
            async fn on_message(mut self, cmd: Map<string, unknown>) {
                reply(await cmd.me.send({ me: cmd.me }))
            }
        }
        async fn main() {
            let svc = spawn.process Relay { n: 0 }
            let r = await Task::timeout(svc.send({ me: svc }), 50)
            match r {
                Result::Ok(v) => print(v)
                Result::Err(e) => print(e.message)
            }
            svc.stop()
        }
        "#);
    assert_eq!(out, vec!["timed out"]);
}

/// `await std::time::sleep(ms)` suspends the body until the deadline passes and
/// yields `unit` (spec `builtins/intrinsic-functions.md`): the coroutine
/// resumes with nothing to print, then continues.
#[test]
fn time_sleep_settles_after_the_delay() {
    let out = run(r#"
        async fn main() {
            await std::time::sleep(10)
            print("slept")
        }
        "#);
    assert_eq!(out, vec!["slept"]);
}
