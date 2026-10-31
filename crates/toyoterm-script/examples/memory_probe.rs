//! Opt-in process/heap probe. Send `start PATH`, `bars N`, `eval N`, `reload N`,
//! `gc`, or `quit` on stdin; sample this PID externally between JSON responses.
//! The counting allocator perturbs timing; use performance_script_roundtrip for
//! latency measurements without allocation instrumentation. No children run.
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use toyoterm_api::{PaneId, TabId, WindowId, WorkspaceId};
use toyoterm_script::*;

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: All pointers/layouts are passed unchanged to System; accounting never
// allocates. Realloc adjusts the counter only after successful reallocation.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                allocated(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn context() -> ScriptContext {
    ScriptContext {
        model: Arc::new(ScriptObjectModel {
            current_workspace: WorkspaceId(1),
            current_window: WindowId(2),
            current_tab: TabId(3),
            current_pane: PaneId(4),
            workspaces: vec![ScriptWorkspace {
                id: WorkspaceId(1),
                name: "Probe".into(),
                windows: vec![WindowId(2)],
            }],
            windows: vec![ScriptWindow {
                id: WindowId(2),
                tabs: vec![TabId(3)],
            }],
            tabs: vec![ScriptTab {
                id: TabId(3),
                title: "Probe".into(),
                panes: vec![PaneId(4)],
                zoomed: false,
            }],
            panes: vec![ScriptPane {
                id: PaneId(4),
                title: "Probe".into(),
                icon_title: None,
                cwd: Some(std::env::current_dir().unwrap().display().to_string()),
                remote_host: None,
                shell_integration_version: None,
                shell_integration_shell: None,
                user_vars: vec![],
                pid: None,
                command_running: false,
                last_exit_status: None,
                screen_text: "prompt>".into(),
                zoomed: false,
            }],
        }),
        handles: vec![
            WorkspaceId(1).into(),
            WindowId(2).into(),
            TabId(3).into(),
            PaneId(4).into(),
        ]
        .into(),
        clipboard: None,
    }
}

fn report(phase: &str, count: usize, started: Instant, mut times: Vec<f64>) {
    times.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        times
            .get((times.len() * percent / 100).min(times.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "{{\"phase\":\"{phase}\",\"count\":{count},\"elapsed_ms\":{},\"p50_ms\":{},\"p95_ms\":{},\"heap_live_bytes\":{},\"heap_peak_bytes\":{}}}",
        started.elapsed().as_secs_f64() * 1000.0,
        percentile(50),
        percentile(95),
        LIVE.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed)
    );
    io::stdout().flush().unwrap();
}

fn main() {
    if std::env::args().any(|arg| arg == "--gc-only") {
        let mut engine = steel::steel_vm::engine::Engine::new();
        engine
            .compile_and_run_raw_program(
                "(define (probe-gc) (#%gc-collect)) (define (probe-nop) void)",
            )
            .unwrap();
        for operation in ["probe-nop", "probe-gc"] {
            for _ in 0..22 {
                PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
                let begin = Instant::now();
                engine
                    .call_function_by_name_with_args(operation, vec![])
                    .unwrap();
                report(operation, 1, begin, vec![]);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        return;
    }
    let ctx = context();
    let (tx, rx) = mpsc::channel();
    let mut worker: Option<ScriptThread> = None;
    let mut id = 0;
    report("ready", 0, Instant::now(), vec![]);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (operation, argument) = line.split_once(' ').unwrap_or((&line, ""));
        if operation == "quit" {
            break;
        }
        PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
        let started = Instant::now();
        if operation == "start" {
            assert!(worker.is_none());
            let sender = tx.clone();
            let (thread, startup) = ScriptThread::start(Some(argument.into()), move |completion| {
                sender.send(completion).unwrap();
            })
            .unwrap();
            assert!(startup.config_error.is_none(), "{:?}", startup.config_error);
            drop(startup);
            worker = Some(thread);
            report(operation, 1, started, vec![]);
            continue;
        }
        let count = if argument.is_empty() {
            1
        } else {
            argument.parse::<usize>().unwrap()
        };
        let mut times = Vec::with_capacity(count);
        for _ in 0..count {
            let invocation = match operation {
                "bars" => ScriptInvocation::Bar {
                    position: StatusBarPosition::Top,
                },
                "eval" => ScriptInvocation::Eval("(+ 1 2)".into()),
                "reload" => ScriptInvocation::Reload,
                "gc" => ScriptInvocation::Eval("(#%gc-collect) void".into()),
                _ => panic!("unknown operation: {operation}"),
            };
            let begin = Instant::now();
            id += 1;
            worker
                .as_ref()
                .unwrap()
                .submit(ScriptRequest {
                    id,
                    context: ctx.clone(),
                    invocation,
                })
                .unwrap();
            let completed = rx.recv_timeout(Duration::from_secs(120)).unwrap();
            assert_eq!(completed.id, id);
            let result = completed.result.unwrap();
            // The app normally executes these outside the VM. Do not spawn
            // external processes in a VM-only memory measurement.
            assert!(result.async_requests.is_empty());
            drop(result);
            times.push(begin.elapsed().as_secs_f64() * 1000.0);
        }
        report(operation, count, started, times);
    }
    drop(worker);
    report("dropped", 0, Instant::now(), vec![]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_success_and_failure_requests_keep_allocations_bounded() {
        let path = std::env::temp_dir().join(format!(
            "toyoterm-memory-regression-{}.scm",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"
          (for-each (lambda (i)
            (theme! (string-append "memory-" (number->string i)) (hash)))
            (range 0 463))
        "#,
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let (worker, startup) = ScriptThread::start(Some(path.clone()), move |completion| {
            tx.send(completion).unwrap();
        })
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(startup.config_error.is_none());
        drop(startup);
        let ctx = context();
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        for id in 1..=1000 {
            let failing = id % 2 == 0;
            let invocation = if failing {
                ScriptInvocation::Eval("(error \"expected allocation regression failure\")".into())
            } else {
                ScriptInvocation::Bar {
                    position: StatusBarPosition::Top,
                }
            };
            worker
                .submit(ScriptRequest {
                    id,
                    context: ctx.clone(),
                    invocation,
                })
                .unwrap();
            let completed = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            assert_eq!(completed.id, id);
            assert_eq!(completed.result.is_err(), failing);
            let peak = PEAK.load(Ordering::Relaxed);
            // Ordinary requests must not re-encode all unchanged themes or
            // create the ~700 MiB forced-GC free-list expansion.
            assert!(
                peak < baseline + 64 * 1024 * 1024,
                "request={id}, baseline={baseline}, peak={peak}"
            );
        }
    }
}
