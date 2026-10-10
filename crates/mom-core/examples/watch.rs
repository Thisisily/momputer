//! Mimics the widget: one long-lived Sampler; a process starts after the first look.
fn main() {
    let mut sampler = mom_core::procs::Sampler::new();
    let _ = sampler.sample();
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "ping -n 30 127.0.0.1 >nul"])
        .env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_SESSION_ID", "watch-test")
        .spawn()
        .unwrap();
    for i in 1..=3 {
        std::thread::sleep(std::time::Duration::from_millis(if i == 1 { 50 } else { 1500 }));
        let t = sampler.sample();
        match t.get(child.id()) {
            Some(p) => println!("sample {i}: pid {} cmd={:?} env={:?} cwd={:?} exe={:?}", p.pid, p.cmd, p.env, p.cwd, p.exe),
            None => println!("sample {i}: not in table"),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
