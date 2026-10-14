//! What is this process, and what should mom call it?
//!
//! Turns `node "C:\proj\node_modules\.bin\..\vite\bin\vite.js" --port 5273` into "vite", and
//! `sh.exe /c/Users/me/AppData/Roaming/npm/bun run dev` into "bun run dev".

use crate::agents::Role;
use crate::procs::Proc;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Node,
    Bun,
    Deno,
    Python,
    Browser,
    Database,
    Vm,
    Tunnel,
    Media,
    Java,
    Dotnet,
    Go,
    Rust,
    Ruby,
    Php,
    Tool,
    Shell,
}

impl Kind {
    /// A real runtime, as opposed to a shell or a little utility like `tail`.
    pub fn is_runtime(self) -> bool {
        !matches!(self, Kind::Shell | Kind::Tool)
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Node => "node",
            Kind::Bun => "bun",
            Kind::Deno => "deno",
            Kind::Python => "python",
            Kind::Browser => "browser",
            Kind::Database => "database",
            Kind::Vm => "vm",
            Kind::Tunnel => "tunnel",
            Kind::Media => "media",
            Kind::Java => "java",
            Kind::Dotnet => "dotnet",
            Kind::Go => "go",
            Kind::Rust => "rust",
            Kind::Ruby => "ruby",
            Kind::Php => "php",
            Kind::Tool => "tool",
            Kind::Shell => "shell",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    DevServer,
    Server,
    Watcher,
    HeadlessBrowser,
    McpServer,
    TestRunner,
    Tunnel,
    Database,
    LogFollower,
    Sleeper,
    Build,
    MediaJob,
    Vm,
    Repl,
    Script,
    Shell,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::DevServer => "dev server",
            Category::Server => "server",
            Category::Watcher => "watcher",
            Category::HeadlessBrowser => "headless browser",
            Category::McpServer => "MCP server",
            Category::TestRunner => "test runner",
            Category::Tunnel => "tunnel",
            Category::Database => "database",
            Category::LogFollower => "tail -f",
            Category::Sleeper => "sleep",
            Category::Build => "build",
            Category::MediaJob => "media job",
            Category::Vm => "VM",
            Category::Repl => "REPL",
            Category::Script => "script",
            Category::Shell => "shell",
        }
    }
}

const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "chromium.exe",
    "chrome-headless-shell.exe",
    "headless_shell.exe",
    "firefox.exe",
    "brave.exe",
];

/// Browsers count only when a robot is driving them. Your own browser is none of mom's business.
pub fn is_automated_browser(p: &Proc) -> bool {
    let name = p.lname();
    if matches!(name.as_str(), "chromedriver.exe" | "msedgedriver.exe" | "geckodriver.exe") {
        return true;
    }
    if !BROWSERS.contains(&name.as_str()) || p.cmd.iter().any(|a| a.starts_with("--type=")) {
        return false;
    }
    if name == "chrome-headless-shell.exe" || name == "headless_shell.exe" {
        return true;
    }
    let exe = p.lexe();
    if exe.contains("ms-playwright") || exe.contains("puppeteer") || exe.contains("\\chrome-for-testing") {
        return true;
    }
    p.cmd.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        a.starts_with("--headless")
            || a.starts_with("--remote-debugging-port")
            || a == "--remote-debugging-pipe"
            || a == "--enable-automation"
            || a.starts_with("-marionette")
            || (a.starts_with("--user-data-dir=")
                && (a.contains("playwright") || a.contains("puppeteer") || a.contains("\\temp\\")))
    })
}

fn is_tunnel_ssh(p: &Proc) -> bool {
    p.cmd.iter().any(|a| a == "-L" || a == "-R" || a == "-D" || a == "-N")
}

pub fn kind_of(p: &Proc) -> Option<Kind> {
    let name = p.lname();
    let stem = name.trim_end_matches(".exe");
    let exe = p.lexe();
    let k = match stem {
        "node" | "node_repl" | "nodemon" => Kind::Node,
        "bun" | "bunx" => Kind::Bun,
        "deno" => Kind::Deno,
        "python" | "pythonw" | "python3" | "py" | "pyw" | "uv" | "uvx" | "uvicorn" | "streamlit"
        | "jupyter" | "jupyter-lab" | "jupyter-notebook" | "celery" | "gunicorn" | "hypercorn"
        | "flask" | "gradio" => Kind::Python,
        s if s.starts_with("python3.") => Kind::Python,
        "chromedriver" | "msedgedriver" | "geckodriver" => Kind::Browser,
        _ if BROWSERS.contains(&name.as_str()) => {
            if is_automated_browser(p) {
                Kind::Browser
            } else {
                return None;
            }
        }
        "postgres" | "pg_ctl" | "mysqld" | "mariadbd" | "mongod" | "mongos" | "redis-server"
        | "memurai" | "clickhouse" | "clickhouse-server" | "qdrant" | "meilisearch"
        | "typesense-server" | "surreal" | "etcd" | "minio" | "valkey-server" => Kind::Database,
        "vmwp" | "vmmem" | "vmmemwsl" | "vboxheadless" | "virtualboxvm" | "vmware-vmx" => Kind::Vm,
        s if s.starts_with("qemu-system") => Kind::Vm,
        "ngrok" | "cloudflared" | "frpc" | "bore" | "localtunnel" | "lt" => Kind::Tunnel,
        "stripe" if p.cmd.iter().any(|a| a == "listen") => Kind::Tunnel,
        "ssh" if is_tunnel_ssh(p) => Kind::Tunnel,
        "ffmpeg" | "ffprobe" | "ffplay" | "magick" | "handbrakecli" | "yt-dlp" | "sox" | "whisper"
        | "whisper-cli" | "rife" | "realesrgan-ncnn-vulkan" => Kind::Media,
        "blender" => Kind::Media,
        "java" | "javaw" => Kind::Java,
        "dotnet" => Kind::Dotnet,
        "go" | "gopls" | "air" => Kind::Go,
        "cargo" | "rustc" | "cargo-watch" | "trunk" if exe.contains(".cargo") || exe.contains(".rustup") || exe.is_empty() => Kind::Rust,
        "ruby" | "rails" | "puma" => Kind::Ruby,
        "php" | "php-cgi" => Kind::Php,
        "esbuild" | "turbo" | "workerd" | "ollama" | "llama-server" | "adb" | "watchexec" | "tail"
        | "sleep" | "watch" | "caddy" | "nginx" | "httpd" | "miniserve" | "mkcert" | "make"
        | "mingw32-make" | "tsc" | "vite" | "wrangler" | "playwright" | "supabase" => Kind::Tool,
        "bash" | "sh" | "zsh" | "dash" | "fish" | "cmd" | "powershell" | "pwsh" | "nu" => Kind::Shell,
        _ => {
            // Console entry points installed into a Python venv (e.g. `.venv\Scripts\blender-mcp.exe`).
            if exe.contains("\\scripts\\") && (exe.contains("\\.venv\\") || exe.contains("\\venv\\")) {
                Kind::Python
            } else {
                return None;
            }
        }
    };
    Some(k)
}

/// Something shipped inside an app (Adobe's private node.exe, Blender's python): not a dev runtime.
pub fn is_app_bundled(p: &Proc) -> bool {
    let exe = p.lexe();
    if exe.is_empty() {
        return false;
    }
    let in_program_files = exe.starts_with("c:\\program files\\")
        || exe.starts_with("c:\\program files (x86)\\")
        || exe.contains("\\windowsapps\\")
        || std::env::var("ProgramFiles")
            .map(|pf| !pf.is_empty() && exe.starts_with(&crate::util::norm_path(&pf)))
            .unwrap_or(false);
    if !in_program_files {
        return false;
    }
    const DEV_HOMES: &[&str] = &[
        "\\nodejs\\", "\\python", "\\git\\", "\\docker\\", "\\deno", "\\bun", "\\nvm", "\\java", "\\jdk",
        "\\eclipse adoptium\\", "\\dotnet\\", "\\go\\", "\\ruby", "\\php", "\\postgresql\\", "\\mongodb",
        "\\redis", "\\memurai", "\\mysql", "\\mariadb", "\\ffmpeg", "\\imagemagick", "\\oracle\\virtualbox",
        "\\vmware\\", "\\qemu", "\\ngrok", "\\cloudflared", "\\stripe", "\\nginx", "\\caddy", "\\ollama",
        "\\blender foundation\\", "\\gstreamer",
    ];
    !DEV_HOMES.iter().any(|d| exe.contains(d))
}

/// Windows' own plumbing. Mom never touches these.
pub fn is_critical(p: &Proc) -> bool {
    matches!(
        p.lname().as_str(),
        "system" | "idle" | "smss.exe" | "csrss.exe" | "wininit.exe" | "winlogon.exe" | "services.exe"
            | "lsass.exe" | "svchost.exe" | "dwm.exe" | "explorer.exe" | "fontdrvhost.exe" | "sihost.exe"
            | "taskhostw.exe" | "runtimebroker.exe" | "searchhost.exe" | "startmenuexperiencehost.exe"
            | "ctfmon.exe" | "registry" | "memory compression" | "lsaiso.exe" | "wudfhost.exe"
            | "audiodg.exe" | "spoolsv.exe" | "conhost.exe" | "openconsole.exe" | "windowsterminal.exe"
    )
}

// ---------------------------------------------------------------- naming

fn arg_lower(a: &str) -> String {
    a.replace('\\', "/").to_ascii_lowercase()
}

/// Resolves `a/b/../c` so `node_modules/.bin/../vite/bin/vite.js` reads as `node_modules/vite/...`.
fn squash(path: &str) -> String {
    let p = path.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" if !out.is_empty() => {}
            "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// Package name for a script inside node_modules: `.../node_modules/@scope/pkg/bin/x.js` -> `@scope/pkg`.
fn package_of(script: &str) -> Option<String> {
    let s = squash(script);
    let idx = s.rfind("node_modules/")?;
    let rest = &s[idx + "node_modules/".len()..];
    let mut parts = rest.split('/');
    let first = parts.next()?;
    if first.starts_with('@') {
        let second = parts.next()?;
        Some(format!("{first}/{second}"))
    } else if first == ".bin" {
        parts.next().map(|s| s.to_string())
    } else {
        Some(first.to_string())
    }
}

fn short_package(pkg: &str) -> String {
    match pkg {
        "@mintlify/cli" | "mint" => "mint".into(),
        "@angular/cli" => "ng".into(),
        "@remix-run/dev" => "remix".into(),
        "@sveltejs/kit" => "svelte-kit".into(),
        "@storybook/cli" | "storybook" => "storybook".into(),
        "@expo/cli" | "expo" => "expo".into(),
        "@playwright/test" | "playwright" | "playwright-core" => "playwright".into(),
        p if p.starts_with("@modelcontextprotocol/") => p.trim_start_matches("@modelcontextprotocol/").into(),
        p => p.to_string(),
    }
}

/// Words that follow a tool name and are worth keeping: `next dev`, `wrangler dev`, `npm run dev`.
fn tail_words(args: &[String], max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() && out.len() < max {
        let a = &args[i];
        if a.starts_with('-') {
            // `--filter @scope/app` style flags keep their value next to them.
            if (a == "--filter" || a == "-F" || a == "--workspace" || a == "-w" || a == "--prefix")
                && i + 1 < args.len()
            {
                out.push(format!("{} {}", a, args[i + 1]));
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(crate::util::file_name(a).to_string());
        i += 1;
    }
    out
}

/// A script path made readable relative to the process's working directory.
fn pretty_script(script: &str, cwd: Option<&str>) -> String {
    let s = squash(script);
    if let Some(cwd) = cwd {
        let c = squash(cwd).to_ascii_lowercase();
        let l = s.to_ascii_lowercase();
        if !c.is_empty() && l.starts_with(&c) && l.len() > c.len() + 1 {
            return s[c.len() + 1..].to_string();
        }
    }
    // MSYS paths like /c/Users/... or absolute Windows paths: keep the last two segments.
    let segs: Vec<&str> = s.split('/').filter(|x| !x.is_empty()).collect();
    if segs.len() >= 2 && (s.starts_with('/') || s.chars().nth(1) == Some(':')) {
        return segs[segs.len() - 2..].join("/");
    }
    s
}

/// Skips interpreter flags (and the values of flags that take one), returning the remaining args.
fn after_flags<'a>(args: &'a [String], with_value: &[&str]) -> &'a [String] {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if !a.starts_with('-') || a == "-" {
            break;
        }
        if with_value.contains(&a.as_str()) {
            i += 2;
        } else {
            i += 1;
        }
    }
    &args[i.min(args.len())..]
}

fn name_from_js(args: &[String], cwd: Option<&str>, runtime: &str) -> String {
    if args.iter().take(3).any(|a| matches!(a.as_str(), "-e" | "--eval" | "-p" | "--print")) {
        return format!("{runtime} -e …");
    }
    let rest = after_flags(args, &["-r", "--require", "--import", "--loader", "--env-file", "--inspect-port", "--max-old-space-size"]);
    let Some(first) = rest.first() else {
        return runtime.to_string();
    };
    let first_l = arg_lower(first);
    if runtime == "bun" {
        match first_l.as_str() {
            "run" | "x" | "test" | "build" | "install" | "add" | "create" => {
                let more = tail_words(after_flags(&rest[1..], &[]), 3);
                if first_l == "x" {
                    return more.first().cloned().unwrap_or_else(|| "bunx".into());
                }
                return format!("bun {} {}", first_l, more.join(" ")).trim().to_string();
            }
            _ => {}
        }
    }
    if first_l == "-e" || first_l == "--eval" {
        return format!("{runtime} -e …");
    }
    let script = first;
    if let Some(pkg) = package_of(script) {
        let tool = short_package(&pkg);
        return match tool.as_str() {
            "npm" => {
                // npx-cli.js runs a package; npm-cli.js runs a script.
                if first_l.contains("npx-cli") {
                    let target = rest[1..].iter().find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
                    let target = target.rsplit_once('@').filter(|(a, _)| !a.is_empty()).map(|(a, _)| a.to_string()).unwrap_or(target);
                    format!("{} (npx)", short_package(&target))
                } else {
                    format!("npm {}", tail_words(&rest[1..], 3).join(" ")).trim().to_string()
                }
            }
            "pnpm" | "yarn" => format!("{} {}", tool, tail_words(&rest[1..], 3).join(" ")).trim().to_string(),
            _ => {
                let words = tail_words(&rest[1..], 1);
                let sub = words.first().filter(|w| {
                    matches!(
                        w.as_str(),
                        "dev" | "start" | "serve" | "preview" | "build" | "watch" | "test" | "develop" | "run" | "studio" | "deploy" | "listen"
                    )
                });
                match sub {
                    Some(s) => format!("{tool} {s}"),
                    None => tool,
                }
            }
        };
    }
    let mut name = pretty_script(script, cwd);
    if runtime == "bun" && args.iter().any(|a| a == "--watch" || a == "--hot") {
        name.push_str(" (watch)");
    }
    name
}

fn name_from_python(p: &Proc, args: &[String], cwd: Option<&str>) -> String {
    let stem = p.lname().trim_end_matches(".exe").to_string();
    if !matches!(stem.as_str(), "python" | "pythonw" | "python3" | "py" | "pyw") && !stem.starts_with("python3.") {
        if stem == "uv" || stem == "uvx" {
            let rest: Vec<String> = after_flags(args, &["--directory", "--project", "--python", "--with", "-p"]).to_vec();
            let rest = if rest.first().map(|s| s == "run" || s == "tool").unwrap_or(false) {
                after_flags(&rest[1..], &["--with", "--python", "-p", "--from"]).to_vec()
            } else {
                rest
            };
            return match rest.first() {
                Some(t) => format!("{} (uv)", crate::util::file_name(t).trim_end_matches(".py")),
                None => stem,
            };
        }
        return format!("{} {}", stem, tail_words(args, 2).join(" ")).trim().to_string();
    }
    let rest = after_flags(args, &["-W", "-X", "-c"]);
    if let Some(pos) = args.iter().position(|a| a == "-m") {
        let module = args.get(pos + 1).cloned().unwrap_or_default();
        let more = tail_words(&args[(pos + 2).min(args.len())..], 1);
        return format!("{} {}", module, more.join(" ")).trim().to_string();
    }
    if args.iter().any(|a| a == "-c") {
        return "python -c …".into();
    }
    match rest.first() {
        Some(script) => {
            let s = pretty_script(script, cwd);
            if s.ends_with("manage.py") && rest.iter().any(|a| a == "runserver") {
                "django runserver".into()
            } else if s.to_ascii_lowercase().ends_with(".exe") {
                crate::util::file_name(&s).trim_end_matches(".exe").to_string()
            } else {
                s
            }
        }
        None => "python".into(),
    }
}

/// `sh.exe /c/Users/me/AppData/Roaming/npm/bun run dev` -> "bun run dev";
/// `cmd /d /s /c "npx -y pkg"` -> "npx -y pkg".
/// The command inside Claude Code's shell wrapper: `... && eval 'CMD' \< /dev/null && pwd -P >| ...`.
fn wrapped_eval(inner: &str) -> Option<String> {
    let e = inner.find("eval '")?;
    let rest = &inner[e + "eval '".len()..];
    // Single-quoted shell string: `'\''` is an escaped quote inside it.
    let unescaped = rest.replace("'\\''", "\u{1}");
    let end = unescaped.find('\'').unwrap_or(unescaped.len());
    let cmd = unescaped[..end].replace('\u{1}', "'");
    let cmd = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cmd.is_empty()).then_some(cmd)
}

fn name_from_shell(args: &[String]) -> String {
    let joined = format!(" {}", args.join(" "));
    let lower = joined.to_ascii_lowercase();
    for marker in [" /c ", " -c ", " -command ", " /k "] {
        if let Some(i) = lower.find(marker) {
            let inner = joined[i + marker.len()..].trim().trim_matches('"').to_string();
            if let Some(cmd) = wrapped_eval(&inner) {
                return crate::util::ellipsize(&cmd, 60);
            }
            if inner.starts_with("source ") || inner.starts_with("-l ") || inner.starts_with("export ") {
                return "shell".into();
            }
            return crate::util::ellipsize(&inner, 60);
        }
    }
    // npm shim scripts run by sh: first arg is the shim path, the rest are the real command.
    if let Some(first) = args.first() {
        let f = arg_lower(first);
        if f.contains("/npm/") || f.ends_with(".sh") || f.contains("/bin/") {
            let tool = crate::util::file_name(first).trim_end_matches(".cmd").trim_end_matches(".ps1");
            let rest: Vec<&str> = args[1..].iter().map(String::as_str).take(4).collect();
            return format!("{} {}", tool, rest.join(" ")).trim().to_string();
        }
    }
    "shell".into()
}

pub fn display_name(p: &Proc) -> String {
    let kind = kind_of(p);
    let args: Vec<String> = p.cmd.iter().skip(1).cloned().collect();
    let cwd = p.cwd.as_deref();
    let stem = p.lname().trim_end_matches(".exe").to_string();
    let name = match kind {
        Some(Kind::Node) if stem == "node_repl" => "node REPL".to_string(),
        Some(Kind::Node) => name_from_js(&args, cwd, "node"),
        Some(Kind::Bun) => name_from_js(&args, cwd, "bun"),
        Some(Kind::Deno) => {
            let sub = args.first().map(|a| a.as_str()).unwrap_or("");
            if sub == "task" {
                format!("deno task {}", tail_words(&args[1..], 1).join(" ")).trim().to_string()
            } else if matches!(sub, "run" | "serve" | "test") {
                name_from_js(&args[1..], cwd, "deno")
            } else {
                name_from_js(&args, cwd, "deno")
            }
        }
        Some(Kind::Python) => name_from_python(p, &args, cwd),
        Some(Kind::Browser) => {
            let exe = p.lexe();
            let who = if exe.contains("ms-playwright") {
                "Playwright "
            } else if exe.contains("puppeteer") {
                "Puppeteer "
            } else {
                ""
            };
            let what = match stem.as_str() {
                "msedge" => "Edge",
                "firefox" => "Firefox",
                "chromedriver" | "msedgedriver" | "geckodriver" => return stem,
                _ => "Chrome",
            };
            let headless = p.cmd.iter().any(|a| a.starts_with("--headless")) || stem.contains("headless");
            format!("{who}{}{what}", if headless { "headless " } else { "automated " })
        }
        Some(Kind::Shell) => name_from_shell(&args),
        Some(Kind::Tool) if stem == "tail" => {
            let file = args.iter().rev().find(|a| !a.starts_with('-') && !a.starts_with('+')).map(|f| crate::util::file_name(f).to_string());
            format!("tail -f {}", file.unwrap_or_default()).trim().to_string()
        }
        Some(Kind::Media) if stem == "ffmpeg" => {
            let out = args.last().map(|o| crate::util::file_name(o).to_string()).unwrap_or_default();
            format!("ffmpeg → {out}")
        }
        Some(_) => {
            let words = tail_words(&args, 2);
            format!("{} {}", stem, words.join(" ")).trim().to_string()
        }
        None => stem,
    };
    crate::util::ellipsize(&name, 48)
}

pub fn mentions_mcp(cmd: &str) -> bool {
    cmd.contains("mcp-server")
        || cmd.contains("modelcontextprotocol")
        || cmd.contains("server-pdf")
        || cmd.contains("fastmcp")
        || cmd.contains("mcp-remote")
        || cmd.contains("-mcp")
        || cmd.contains("_mcp")
        || cmd.contains("mcp_")
        || cmd.contains("mcp/")
        || cmd.contains("/mcp")
        || cmd.contains("mcp.")
        || cmd.contains(" mcp")
}

const DEV_TOOLS: &[&str] = &[
    "vite", "next dev", "next start", "nuxt", "astro", "remix", "webpack serve", "webpack-dev-server",
    "parcel", "svelte-kit", "storybook", "expo", "mint dev", "mintlify", "docusaurus", "gatsby develop",
    "wrangler dev", "http-server", "live-server", "serve ", "http.server", "uvicorn", "flask run",
    "runserver", "streamlit", "gradio", "jupyter", "run dev", "run start", "run serve", "run preview",
    " dev", "serve.ts", "server.ts", "server.js", "server.mjs", "--hot",
];

pub fn category_of(p: &Proc, has_ports: bool, role: Role) -> Category {
    let kind = kind_of(p);
    let cmd = p.cmdline().to_ascii_lowercase().replace('\\', "/");
    let name = display_name(p).to_ascii_lowercase();
    match kind {
        Some(Kind::Browser) => return Category::HeadlessBrowser,
        Some(Kind::Database) => return Category::Database,
        Some(Kind::Vm) => return Category::Vm,
        Some(Kind::Tunnel) => return Category::Tunnel,
        Some(Kind::Media) => return Category::MediaJob,
        Some(Kind::Shell) => return Category::Shell,
        _ => {}
    }
    if p.lname().starts_with("node_repl") {
        return Category::Repl;
    }
    if mentions_mcp(&cmd) || (role == Role::Harness && !has_ports) {
        return Category::McpServer;
    }
    if (p.lname() == "tail.exe" || cmd.starts_with("tail "))
        && p.cmd.iter().any(|a| a.starts_with('-') && a.to_ascii_lowercase().contains('f')) {
            return Category::LogFollower;
        }
    if p.lname() == "sleep.exe" {
        return Category::Sleeper;
    }
    let watching = p.cmd.iter().any(|a| a == "--watch" || a == "-w" || a == "watch")
        || cmd.contains("nodemon")
        || cmd.contains("tsx watch")
        || cmd.contains("cargo-watch");
    let dev = DEV_TOOLS.iter().any(|t| name.contains(t) || cmd.contains(t));
    if dev && (has_ports || !watching) {
        return Category::DevServer;
    }
    if watching {
        return Category::Watcher;
    }
    if has_ports {
        return Category::Server;
    }
    if cmd.contains("playwright test") || cmd.contains("vitest") || cmd.contains("jest") || cmd.contains("pytest") || cmd.contains("mocha") {
        return Category::TestRunner;
    }
    if cmd.contains(" build") || cmd.contains("tsc") || cmd.contains("esbuild") {
        return Category::Build;
    }
    Category::Script
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::Proc;

    fn p(name: &str, cmd: &[&str]) -> Proc {
        Proc {
            pid: 1,
            name: name.into(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn names_from_the_wild() {
        // All of these were found running (and forgotten) on a real machine.
        let cases: Vec<(Proc, &str)> = vec![
            (p("node.exe", &["node", r"C:\Workspace\storefront\node_modules\vite\bin\vite.js"]), "vite"),
            (p("node.exe", &["node", r"C:\Workspace\Nimbus\node_modules\.bin\\..\vite\bin\vite.js", "--host", "127.0.0.1", "--port", "5273", "--strictPort"]), "vite"),
            (p("node.exe", &[r"C:\Program Files\nodejs\node.exe", r"C:\Users\me\AppData\Roaming\npm/node_modules/mint/index.js", "dev", "--port", "3333", "--no-open"]), "mint dev"),
            (p("node.exe", &["node", "--no-deprecation", r"C:\npm\node_modules\mint\node_modules\@mintlify\cli\bin\start.js", "dev", "--port", "3333"]), "mint dev"),
            (p("node.exe", &["node", r"C:\Users\me\AppData\Roaming\npm\\node_modules\npm\bin\npx-cli.js", "-y", "@modelcontextprotocol/server-pdf", "--stdio"]), "server-pdf (npx)"),
            (p("node.exe", &["node", r"C:\Users\me\AppData\Roaming\npm\\node_modules\pnpm\bin\pnpm.cjs", "--filter", "@atlas/web", "dev"]), "pnpm --filter @atlas/web dev"),
            (p("bun.exe", &[r"C:\npm\node_modules\bun\bin\bun.exe", "--no-env-file", "scripts/preview-setup.ts"]), "scripts/preview-setup.ts"),
            (p("bun.exe", &[r"C:\npm\node_modules\bun\bin\bun.exe", "run", "dev"]), "bun run dev"),
            (p("bun.exe", &[r"C:\npm\node_modules\bun\bin\bun.exe", "--watch", "server/index.ts"]), "server/index.ts (watch)"),
            (p("bun.exe", &[r"C:\npm\node_modules\bun\bin\bun.exe", "x", "--no-install", "vite"]), "vite"),
            (p("node.exe", &["node", "C:/Users/me/.claude/mcp-servers/codex-imagegen/index.js"]), "codex-imagegen/index.js"),
            (p("node.exe", &["node", "-e", "require('http').createServer().listen(1)"]), "node -e …"),
            (p("python.exe", &["python", "-m", "http.server", "8000"]), "http.server 8000"),
            (p("uv.exe", &["uv", "--directory", r"C:\ext\blender-mcp", "run", "blender-mcp"]), "blender-mcp (uv)"),
            (p("tail.exe", &[r"C:\Program Files\Git\usr\bin\tail.exe", "-n", "+1", "-f", "data/studio-proof/proof-run-beta-2.log"]), "tail -f proof-run-beta-2.log"),
            (p("sh.exe", &[r"C:\Program Files\Git\usr\bin\sh.exe", "/c/Users/me/AppData/Roaming/npm/bun", "run", "dev"]), "bun run dev"),
            (p("cmd.exe", &[r"C:\WINDOWS\system32\cmd.exe", "/d", "/s", "/c", "npx -y @modelcontextprotocol/server-pdf --stdio"]), "npx -y @modelcontextprotocol/server-pdf --stdio"),
            (
                p("bash.exe", &[r"C:\Program Files\Git\bin\bash.exe", "-c", "source /c/Users/me/.claude/shell-snapshots/snapshot-bash-1.sh 2>/dev/null || true && export TEMP='C:\\Temp' && eval 'until grep -q DONE b.log; do sleep 5; done' \\< /dev/null && pwd -P >| /tmp/claude-1-cwd"]),
                "until grep -q DONE b.log; do sleep 5; done",
            ),
            (
                p("bash.exe", &[r"C:\Program Files\Git\bin\bash.exe", "-c", "-l", "export TEMP='C:\\Temp' && eval 'echo '\\''hi'\\'' && npm run dev' \\< /dev/null"]),
                "echo 'hi' && npm run dev",
            ),
        ];
        for (proc_, want) in cases {
            assert_eq!(display_name(&proc_), want, "for {:?}", proc_.cmd);
        }
    }

    #[test]
    fn scripts_are_shown_relative_to_their_folder() {
        let mut x = p("bun.exe", &["bun", r"C:\Workspace\storefront\test-results\answers-serve.ts", "6187"]);
        x.cwd = Some(r"C:\Workspace\storefront".into());
        assert_eq!(display_name(&x), "test-results/answers-serve.ts");
    }

    #[test]
    fn categories() {
        let vite = p("node.exe", &["node", r"C:\proj\node_modules\vite\bin\vite.js"]);
        assert_eq!(category_of(&vite, true, Role::Task), Category::DevServer);
        let watch = p("bun.exe", &["bun", "--watch", "server/index.ts"]);
        assert_eq!(category_of(&watch, false, Role::Task), Category::Watcher);
        let mcp = p("node.exe", &["node", "C:/Users/me/.claude/mcp-servers/codex-imagegen/index.js"]);
        assert_eq!(category_of(&mcp, false, Role::Harness), Category::McpServer);
        let tail = p("tail.exe", &["tail", "-n0", "-F", "x.log"]);
        assert_eq!(category_of(&tail, false, Role::Task), Category::LogFollower);
        let script = p("bun.exe", &["bun", "--no-env-file", "scripts/preview-setup.ts"]);
        assert_eq!(category_of(&script, false, Role::Task), Category::Script);
    }

    #[test]
    fn your_browser_is_not_a_runtime_but_a_robots_is() {
        let mine = p("chrome.exe", &["chrome.exe", "--profile-directory=Default"]);
        assert_eq!(kind_of(&mine), None);
        let robot = p("chrome.exe", &["chrome.exe", "--headless=new", "--remote-debugging-port=9222"]);
        assert_eq!(kind_of(&robot), Some(Kind::Browser));
        assert_eq!(display_name(&robot), "headless Chrome");
        let helper = p("chrome.exe", &["chrome.exe", "--type=renderer", "--headless"]);
        assert_eq!(kind_of(&helper), None);
    }

    #[test]
    fn app_bundled_runtimes_are_left_alone() {
        let mut adobe = p("node.exe", &["node.exe", "main.js"]);
        adobe.exe = Some(r"C:\Program Files\Adobe\Adobe Creative Cloud Experience\libs\node.exe".into());
        assert!(is_app_bundled(&adobe));
        let mut dev = p("node.exe", &["node.exe", "main.js"]);
        dev.exe = Some(r"C:\Program Files\nodejs\node.exe".into());
        assert!(!is_app_bundled(&dev));
        let mut nvm = p("node.exe", &["node.exe"]);
        nvm.exe = Some(r"C:\Users\me\AppData\Roaming\nvm\v22.22.0\node.exe".into());
        assert!(!is_app_bundled(&nvm));
    }
}
