use super::*;

/// The control message passed into an async coroutine. The first resume is
/// `Start`; every later resume delivers the result of whatever the coroutine
/// last awaited (or suspended on).
pub(super) enum Control {
    Start,
    Resume(Result<Value, RunError>),
}

/// One suspended async call. Its promise is shared with every `await`er; the
/// coroutine runs the function body and suspends at each `await`.
pub(super) struct Task {
    coro: Coroutine<Control, (), Result<Value, RunError>, DefaultStack>,
    promise: Rc<RefCell<crate::value::Promise>>,
}

/// Settles one `send` task: fulfills the handle's promise with the reply
/// value, or rejects it when the message failed or the service stopped.
pub(super) type ServiceSettle = Rc<dyn Fn(&Interpreter, Result<Value, RunError>)>;

/// One queued message: the deep-copied message value and the settle function
/// of the `send` that enqueued it.
pub(super) struct ServiceMsg {
    pub(super) message: Value,
    pub(super) settle: ServiceSettle,
}

/// A live service started by `spawn.process Type { … }`: its state object
/// (the deep-copied field initializers, mutated in place by `on_message`),
/// the FIFO message queue, and the busy/stopped flags that gate the pump
/// (spec `concurrency.md`).
pub(super) struct ServiceInstance {
    pub(super) type_name: String,
    /// The service type's `on_message` function, resolved through the spawn
    /// site's scope chain (a module-level struct lives in the module env, not
    /// the interpreter root).
    pub(super) on_message: Option<Value>,
    pub(super) state: Value,
    pub(super) queue: RefCell<VecDeque<ServiceMsg>>,
    pub(super) busy: Cell<bool>,
    pub(super) stopped: Cell<bool>,
}

// The interpreter an async coroutine's closure runs against. Set (to `self`)
// around every `resume`, so the `'static` coroutine closures can reach the
// shared `Interpreter` without capturing a reference. Single-threaded, so the
// raw pointer is always valid while a closure is executing.
thread_local! {
    pub(super) static CURRENT_INTERP: Cell<Option<*const Interpreter>> = const { Cell::new(None) };
}

impl Interpreter {
    /// Start an async function body as a coroutine task and inline-resume it
    /// once, so its synchronous prefix runs at the call site exactly as in JS.
    /// Returns the task's promise.
    /// Suspend the current task until `promise` settles, resolving to its
    /// value (or failing with its rejection). Shared by `await` expressions
    /// and the service pump's `on_message` chaining.
    pub(super) fn await_promise(
        &self,
        promise: Rc<RefCell<crate::value::Promise>>,
        span: std::ops::Range<usize>,
    ) -> Result<Value, RunError> {
        let task = self.current_task.get().ok_or_else(|| {
            RunError::err(
                "`await` may only be used inside an `async` function",
                span.clone(),
            )
        })?;
        let settled = {
            let promise = promise.borrow();
            match &promise.state {
                crate::value::PromiseState::Pending => None,
                crate::value::PromiseState::Fulfilled(v) => Some(Ok(v.clone())),
                crate::value::PromiseState::Rejected(e) => Some(Err(e.clone())),
            }
        };
        if let Some(result) = settled {
            // Awaiting an already-settled promise defers to a microtask
            // (matching JS), so re-queue ourselves.
            self.ready
                .borrow_mut()
                .push_back((task, Control::Resume(result)));
        } else {
            promise.borrow_mut().awaiters.push_back(task);
        }
        let yielder = self.current_yielder(task);
        let next = unsafe { (*yielder).suspend(()) };
        match next {
            Control::Resume(result) => result,
            Control::Start => {
                unreachable!("a suspended coroutine cannot be resumed with Start")
            }
        }
    }

    /// Run one message through the service's `on_message`: bind `self` to
    /// the service state and `reply` to a cell the pump reads back.
    pub(super) fn invoke_on_message(
        &self,
        func: &Rc<FunctionValue>,
        state: Value,
        message: Value,
        reply_cell: &Rc<RefCell<Option<Value>>>,
    ) -> Result<Value, RunError> {
        let reply = {
            let cell = Rc::clone(reply_cell);
            Value::Native(Rc::new(move |_interp, args| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                *cell.borrow_mut() = Some(v);
                Ok(Value::Null)
            }))
        };
        let bound: Vec<Option<Value>> = func
            .params
            .iter()
            .map(|param| {
                if param.name == "self" {
                    Some(state.clone())
                } else {
                    Some(message.clone())
                }
            })
            .collect();
        if func.is_async {
            let func = func.clone();
            return Ok(self.spawn_async(move |_yielder| {
                with_interp(|interp| {
                    let env = Env::child(&func.closure);
                    interp.bind_callee_args(&env, &func.params, bound)?;
                    env.borrow_mut().define("reply", reply, false);
                    interp.run_body(&func.body, &env, func.return_type.is_some())
                })
            }));
        }
        let env = Env::child(&func.closure);
        self.bind_callee_args(&env, &func.params, bound)?;
        env.borrow_mut().define("reply", reply, false);
        self.run_body(&func.body, &env, func.return_type.is_some())
    }

    /// Settle one message's send task, mark the service idle, and take the
    /// next queued message (arrival order, one at a time).
    pub(super) fn finish_message(
        &self,
        service: &Rc<ServiceInstance>,
        settle: &ServiceSettle,
        reply_cell: &Rc<RefCell<Option<Value>>>,
        result: Result<Value, RunError>,
    ) {
        let outcome = result.map(|_| reply_cell.borrow().clone().unwrap_or(Value::Null));
        settle(self, outcome);
        service.busy.set(false);
        self.service_pump(service);
    }

    /// Process queued messages while the service is idle: one `on_message`
    /// runs to completion (through every `await`) before the next begins.
    pub(super) fn service_pump(&self, service: &Rc<ServiceInstance>) {
        while !service.stopped.get() && !service.busy.get() {
            let Some(msg) = service.queue.borrow_mut().pop_front() else {
                break;
            };
            service.busy.set(true);
            let reply_cell = Rc::new(RefCell::new(None));
            let func = service.on_message.clone();
            let Some(Value::Function(func)) = func else {
                (msg.settle)(
                    self,
                    Err(RunError::err(
                        format!("service `{}` declares no `on_message`", service.type_name),
                        0..0,
                    )),
                );
                service.busy.set(false);
                continue;
            };
            match self.invoke_on_message(&func, service.state.clone(), msg.message, &reply_cell) {
                Ok(Value::Promise(p)) => {
                    // An async `on_message` holds the service busy until it
                    // settles; the next message waits (spec concurrency.md).
                    let service = service.clone();
                    let settle = msg.settle;
                    self.spawn_async(move |_yielder| {
                        with_interp(|interp| {
                            let result = interp.await_promise(p, 0..0);
                            interp.finish_message(&service, &settle, &reply_cell, result);
                            Ok(Value::Null)
                        })
                    });
                    return;
                }
                Ok(_) => {
                    self.finish_message(service, &msg.settle, &reply_cell, Ok(Value::Null));
                }
                Err(e) => {
                    // A failing message settles its send as rejected; the
                    // service keeps handling what is queued.
                    (msg.settle)(self, Err(e));
                    service.busy.set(false);
                }
            }
        }
    }

    pub(super) fn spawn_async(
        &self,
        body: impl FnOnce(&Yielder<Control, ()>) -> Result<Value, RunError> + 'static,
    ) -> Value {
        let promise = Rc::new(RefCell::new(crate::value::Promise {
            state: crate::value::PromiseState::Pending,
            awaiters: VecDeque::new(),
        }));
        let promise_for_task = promise.clone();
        let interp = self as *const Interpreter;
        let coro = Coroutine::with_stack(
            DefaultStack::new(COROUTINE_STACK_SIZE).expect("failed to allocate async task stack"),
            move |yielder: &Yielder<Control, ()>, _start: Control| -> Result<Value, RunError> {
                // SAFETY: `resume_task` sets CURRENT_INTERP to this interpreter
                // around every resume; the coroutine only runs inside one.
                let interp = unsafe { &*interp };
                let id = interp
                    .current_task
                    .get()
                    .expect("a task id is set before an async coroutine runs");
                interp.task_yielder.borrow_mut()[id] = Some(yielder as *const Yielder<Control, ()>);
                body(yielder)
            },
        );
        let id = {
            let mut tasks = self.tasks.borrow_mut();
            let mut free = self.task_free.borrow_mut();
            let mut yielder_slots = self.task_yielder.borrow_mut();
            if let Some(id) = free.pop() {
                // Reuse a slot left by a completed task. Its yielder pointer is
                // stale (the coroutine stack is gone), so reset it to `None`.
                yielder_slots[id] = None;
                tasks[id] = Some(Task {
                    coro,
                    promise: promise_for_task,
                });
                id
            } else {
                tasks.push(Some(Task {
                    coro,
                    promise: promise_for_task,
                }));
                yielder_slots.push(None);
                tasks.len() - 1
            }
        };
        self.resume_task(id, Control::Start);
        Value::Promise(promise)
    }

    /// Resume `id`'s coroutine with `control`. A `Yield` parks the task back in
    /// `tasks`; a `Return` settles its promise and enqueues every awaiter (FIFO)
    /// for the next drive pass. `current_task`/`CURRENT_INTERP` are saved and
    /// restored so nested spawns from inside a coroutine work.
    pub(super) fn resume_task(&self, id: usize, control: Control) {
        let prev = self.current_task.replace(Some(id));
        let prev_interp = CURRENT_INTERP.with(|c| c.replace(Some(self as *const Interpreter)));
        // SAFETY invariant: a coroutine only ever runs inside this method, so
        // while it is executing the interpreter is necessarily alive. Dropping
        // an `Interpreter` drops every task it owns; nothing resumes them
        // afterward. The raw pointer captured by `spawn_async` cannot dangle.
        debug_assert_eq!(
            CURRENT_INTERP.with(|c| c.get()),
            Some(self as *const Interpreter)
        );
        let mut task = self.tasks.borrow_mut()[id].take().expect("resumable task");
        let result = task.coro.resume(control);
        CURRENT_INTERP.with(|c| c.set(prev_interp));
        self.current_task.set(prev);
        match result {
            corosensei::CoroutineResult::Yield(()) => {
                self.tasks.borrow_mut()[id] = Some(task);
            }
            corosensei::CoroutineResult::Return(ret) => {
                let mut promise = task.promise.borrow_mut();
                promise.state = match &ret {
                    Ok(v) => crate::value::PromiseState::Fulfilled(v.clone()),
                    Err(e) => crate::value::PromiseState::Rejected(e.clone()),
                };
                let awaiters = std::mem::take(&mut promise.awaiters);
                drop(promise);
                for awaiter in awaiters {
                    self.ready
                        .borrow_mut()
                        .push_back((awaiter, Control::Resume(ret.clone())));
                }
                // Free the slot for reuse. A returned task is never enqueued in
                // `ready` again (only suspended tasks are), so handing the id
                // to the next `spawn_async` is safe.
                self.task_free.borrow_mut().push(id);
                // Only trim on the *outermost* resume. A nested resume (a task
                // spawned while another coroutine runs) finds every running
                // coroutine's slot "taken" (`None`), so trimming there would
                // truncate live slots. Reuse (`task_free`) keeps the vector
                // bounded meanwhile.
                if prev.is_none() {
                    self.trim_completed_slots();
                }
            }
        }
    }

    /// Shrink `tasks`/`task_yielder` once every trailing slot is a completed
    /// one, and drop free-list ids that are now out of bounds.
    pub(super) fn trim_completed_slots(&self) {
        let mut tasks = self.tasks.borrow_mut();
        if tasks.last().is_some_and(Option::is_none) {
            let mut len = tasks.len();
            while len > 0 && tasks[len - 1].is_none() {
                len -= 1;
            }
            tasks.truncate(len);
            drop(tasks);
            self.task_yielder.borrow_mut().truncate(len);
            self.task_free.borrow_mut().retain(|&id| id < len);
        }
    }

    /// Drain the ready queue (JS microtasks): resume each queued task in FIFO
    /// order until none is ready to run. With no task ready, the timer half of
    /// the loop takes over: fire deadlines that have passed, otherwise sleep
    /// until the next one (`Task::timeout` deadlines), and end when neither
    /// has work left.
    pub(super) fn drive(&self) {
        loop {
            let next = self.ready.borrow_mut().pop_front();
            let Some((id, control)) = next else {
                if self.fire_due_timers() {
                    continue;
                }
                if !self.sleep_to_next_timer() {
                    break;
                }
                continue;
            };
            self.resume_task(id, control);
        }
    }

    /// Settle every timer whose deadline has passed. Returns whether any
    /// fired: a settle wakes awaiters, so the caller re-checks the ready queue.
    pub(super) fn fire_due_timers(&self) -> bool {
        let now = std::time::Instant::now();
        let mut due = Vec::new();
        {
            let mut timers = self.timers.borrow_mut();
            let mut i = 0;
            while i < timers.len() {
                if timers[i].0 <= now {
                    due.push(timers.swap_remove(i));
                } else {
                    i += 1;
                }
            }
        }
        if due.is_empty() {
            return false;
        }
        for (_, promise, value) in due {
            // Settle-once: a promise the waiter already settled is a no-op.
            settle_promise_once(self, &promise, Ok(value));
        }
        true
    }

    /// Wait until the earliest pending deadline, returning `false` when there
    /// is none left to wait for (nothing else will wake the loop). Timers
    /// whose promise settled before their deadline are dropped here — the
    /// wrapped task won the race — so waiting for them cannot stall a program
    /// that is otherwise done.
    pub(super) fn sleep_to_next_timer(&self) -> bool {
        let next = {
            let mut timers = self.timers.borrow_mut();
            timers.retain(|(_, promise, _)| {
                matches!(promise.borrow().state, crate::value::PromiseState::Pending)
            });
            timers.iter().map(|(deadline, _, _)| *deadline).min()
        };
        let Some(next) = next else {
            return false;
        };
        let wait = next.saturating_duration_since(std::time::Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        true
    }

    pub(super) fn current_yielder(&self, id: usize) -> *const Yielder<Control, ()> {
        self.task_yielder.borrow()[id].expect("an awaiting task has a yielder")
    }
}
