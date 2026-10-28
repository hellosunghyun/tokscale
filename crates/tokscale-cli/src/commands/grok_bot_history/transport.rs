use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const HELPER: &str = include_str!("aside.py");
const PROGRAM: &str = include_str!("collector.js");
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    version: u32,
    root: PathBuf,
    pipes: Vec<Pipe>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pipe {
    name: String,
    device: u64,
    inode: u64,
}
const ERROR_CODES: &[&str] = &[
    "owner_unavailable",
    "owner_changed",
    "browser_dashboard_unavailable",
    "baseline_unavailable",
    "baseline_scope_invalid",
    "invalid_range",
    "invalid_page",
    "invalid_cursor",
    "cursor_cycle",
    "duplicate_item",
    "scope_drift",
    "invalid_bot_record",
    "charge_category_unavailable",
    "usage_type_unavailable",
    "token_meters_unavailable",
    "meter_schema_unavailable",
    "browser_runtime_unsupported",
    "browser_range_runtime",
    "browser_owner_runtime",
    "browser_walk_runtime",
    "browser_recheck_runtime",
    "channel_encoding_unavailable",
    "invalid_meter",
    "meter_overflow",
    "total_mismatch",
    "event_outside_range",
    "page_limit",
    "byte_limit",
    "collection_deadline",
    "page_request_deadline",
    "http_transport",
    "collection_deadline_discover",
    "collection_deadline_owner",
    "collection_deadline_walk",
    "collection_deadline_recheck",
    "http_auth",
    "http_permission",
    "http_rate_limit",
    "http_error",
    "http_server",
    "response_not_json",
    "response_schema",
    "private_channel_invalid",
    "cleanup_incomplete",
];

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FailurePhase {
    Discover,
    Owner,
    Walk,
    Recheck,
}
impl FailurePhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Discover => "discover",
            Self::Owner => "owner",
            Self::Walk => "walk",
            Self::Recheck => "recheck",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureDiagnostic {
    http_status: Option<u16>,
    phase: FailurePhase,
    successful_pages: u32,
    elapsed_ms: u64,
}
impl FailureDiagnostic {
    fn parse(value: &serde_json::Value) -> Option<Self> {
        if value.as_object()?.len() != 4 || value.get("http_status").is_none() {
            return None;
        }
        let result: Self = serde_json::from_value(value.clone()).ok()?;
        if result
            .http_status
            .is_some_and(|status| !(100..=599).contains(&status))
            || result.successful_pages > 512
            || result.elapsed_ms > 3_600_000
        {
            return None;
        }
        Some(result)
    }
}
pub(super) fn control_failure(code: &str, diagnostic: Option<&serde_json::Value>) -> anyhow::Error {
    if !ERROR_CODES.contains(&code) {
        return anyhow!("grok_history_browser_transport_unavailable");
    }
    if let Some(info) = diagnostic.and_then(FailureDiagnostic::parse) {
        let status = info
            .http_status
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".into());
        return anyhow!(
            "grok_history_{code} http_status={status} phase={} successful_pages={} elapsed_ms={}",
            info.phase.as_str(),
            info.successful_pages,
            info.elapsed_ms
        );
    }
    anyhow!("grok_history_{code}")
}
struct Guard {
    child: Child,
    output: ChildStdout,
    buffer: Vec<u8>,
    lines: Vec<Vec<u8>>,
    pipes: Vec<(PathBuf, u64, u64)>,
    done: bool,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
        for (path, dev, ino) in &self.pipes {
            if fs::symlink_metadata(path).is_ok_and(|info| {
                info.file_type().is_fifo()
                    && info.uid() == unsafe { libc::geteuid() }
                    && info.dev() == *dev
                    && info.ino() == *ino
            }) {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn nonblocking(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!("grok_history_private_channel_invalid");
    }
    Ok(())
}
impl Guard {
    fn poll(&mut self) -> Result<()> {
        let mut bytes = [0u8; 4096];
        loop {
            match self.output.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    self.buffer.extend_from_slice(&bytes[..n]);
                    if self.buffer.len() > 16384 {
                        bail!("grok_history_aside_control_invalid");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => bail!("grok_history_aside_unavailable"),
            }
        }
        while let Some(end) = self.buffer.iter().position(|v| *v == b'\n') {
            let line: Vec<_> = self.buffer.drain(..=end).collect();
            if line.len() > 4096 || self.lines.len() > 3 {
                bail!("grok_history_aside_control_invalid");
            }
            let value: serde_json::Value = serde_json::from_slice(&line)
                .map_err(|_| anyhow!("grok_history_aside_control_invalid"))?;
            if let Some(ok) = value.get("ok").and_then(|v| v.as_bool()) {
                if !ok {
                    let code = value.get("error").and_then(|v| v.as_str()).unwrap_or("");
                    return Err(control_failure(code, value.get("diagnostic")));
                }
                self.done = true;
            } else {
                self.lines.push(line);
            }
        }
        Ok(())
    }
    fn hello(&mut self, deadline: Instant) -> Result<Hello> {
        loop {
            self.poll()?;
            if !self.lines.is_empty() {
                return serde_json::from_slice(&self.lines.remove(0))
                    .map_err(|_| anyhow!("grok_history_aside_control_invalid"));
            }
            if Instant::now() >= deadline
                || self
                    .child
                    .try_wait()
                    .map_err(|_| anyhow!("grok_history_aside_unavailable"))?
                    .is_some()
            {
                bail!("grok_history_aside_unavailable");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
fn check_fifo_as(info: &fs::Metadata, dev: u64, ino: u64, owner: u32) -> Result<()> {
    if !info.file_type().is_fifo()
        || info.uid() != owner
        || info.nlink() != 1
        || info.mode() & 0o7777 != 0o600
        || info.dev() != dev
        || info.ino() != ino
    {
        bail!("grok_history_private_channel_invalid");
    }
    Ok(())
}
fn check_fifo(info: &fs::Metadata, dev: u64, ino: u64) -> Result<()> {
    check_fifo_as(info, dev, ino, unsafe { libc::geteuid() })
}
fn channel_path(hello: &Hello, name: &str) -> Result<(PathBuf, u64, u64)> {
    if hello.version != 1
        || hello.pipes.len() != 2
        || !hello.root.is_absolute()
        || hello.root.canonicalize().ok().as_ref() != Some(&hello.root)
    {
        bail!("grok_history_private_channel_invalid");
    }
    let info = fs::symlink_metadata(&hello.root)
        .map_err(|_| anyhow!("grok_history_private_channel_invalid"))?;
    if !info.is_dir() || info.uid() != unsafe { libc::geteuid() } || info.mode() & 0o022 != 0 {
        bail!("grok_history_private_channel_invalid");
    }
    if hello.pipes.iter().filter(|pipe| pipe.name == name).count() != 1 {
        bail!("grok_history_private_channel_invalid");
    }
    let pipe = hello
        .pipes
        .iter()
        .find(|pipe| pipe.name == name)
        .ok_or_else(|| anyhow!("grok_history_private_channel_invalid"))?;
    let path = hello.root.join(name);
    check_fifo(
        &fs::symlink_metadata(&path)
            .map_err(|_| anyhow!("grok_history_private_channel_invalid"))?,
        pipe.device,
        pipe.inode,
    )?;
    Ok((path, pipe.device, pipe.inode))
}
fn open_pipe(spec: &(PathBuf, u64, u64), write: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(!write)
        .write(write)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&spec.0)
        .map_err(|_| anyhow!("grok_history_private_channel_invalid"))?;
    check_fifo(
        &file
            .metadata()
            .map_err(|_| anyhow!("grok_history_private_channel_invalid"))?,
        spec.1,
        spec.2,
    )?;
    Ok(file)
}
fn write_options(
    guard: &mut Guard,
    spec: &(PathBuf, u64, u64),
    bytes: &[u8],
    deadline: Instant,
) -> Result<()> {
    let mut file = loop {
        guard.poll()?;
        if Instant::now() >= deadline {
            bail!("grok_history_private_channel_deadline");
        }
        // ENXIO means the supported REPL has not opened its reader yet.
        match OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&spec.0)
        {
            Ok(file) => {
                check_fifo(
                    &file
                        .metadata()
                        .map_err(|_| anyhow!("grok_history_private_channel_invalid"))?,
                    spec.1,
                    spec.2,
                )?;
                break file;
            }
            Err(e) if e.raw_os_error() == Some(libc::ENXIO) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => bail!("grok_history_private_channel_invalid"),
        }
    };
    let mut offset = 0;
    while offset < bytes.len() {
        guard.poll()?;
        if Instant::now() >= deadline {
            bail!("grok_history_private_channel_deadline");
        }
        match file.write(&bytes[offset..]) {
            Ok(0) => bail!("grok_history_private_channel_invalid"),
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => bail!("grok_history_private_channel_invalid"),
        }
    }
    Ok(())
}
fn frame(
    file: &mut File,
    mut poll: impl FnMut() -> Result<bool>,
    deadline: Instant,
) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let done = poll()?;
        if Instant::now() >= deadline {
            bail!("grok_history_private_channel_deadline");
        }
        match file.read(&mut buffer) {
            Ok(0) if !data.is_empty() => {
                if data.len() < 4 {
                    bail!("grok_history_private_frame_invalid");
                }
                let length = u32::from_be_bytes(
                    data[..4]
                        .try_into()
                        .map_err(|_| anyhow!("grok_history_private_frame_invalid"))?,
                ) as usize;
                if length == 0 || length > LIMIT || data.len() != length + 4 {
                    bail!("grok_history_private_frame_invalid");
                }
                return Ok(data.split_off(4));
            }
            Ok(0) if done => bail!("grok_history_private_frame_invalid"),
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(n) => {
                data.extend_from_slice(&buffer[..n]);
                if data.len() > LIMIT + 4 {
                    bail!("grok_history_private_frame_invalid");
                }
                if data.len() >= 4 {
                    let length = u32::from_be_bytes(
                        data[..4]
                            .try_into()
                            .map_err(|_| anyhow!("grok_history_private_frame_invalid"))?,
                    ) as usize;
                    if length == 0 || length > LIMIT || data.len() > length + 4 {
                        bail!("grok_history_private_frame_invalid");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => bail!("grok_history_private_channel_invalid"),
        }
    }
}
fn execute(
    program: &str,
    requested: Option<&Range>,
    max_pages: u32,
    timeout: u32,
) -> Result<Vec<u8>> {
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| anyhow!("grok_history_python3_unavailable"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("grok_history_aside_unavailable"))?;
    nonblocking(output.as_raw_fd())?;
    let mut guard = Guard {
        child,
        output,
        buffer: Vec::new(),
        lines: Vec::new(),
        pipes: Vec::new(),
        done: false,
    };
    let mut stdin = guard
        .child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("grok_history_aside_unavailable"))?;
    let control =
        serde_json::to_vec(&serde_json::json!({"program":program,"timeoutSeconds":timeout}))
            .map_err(|_| anyhow!("grok_history_aside_control_invalid"))?;
    stdin
        .write_all(&control)
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|_| anyhow!("grok_history_aside_unavailable"))?;
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(u64::from(timeout) + 90);
    let hello = guard.hello(Instant::now() + Duration::from_secs(30))?;
    let options = channel_path(&hello, "options.pipe")?;
    guard.pipes.push(options.clone());
    let records = channel_path(&hello, "records.pipe")?;
    guard.pipes.push(records.clone());
    let mut reader = open_pipe(&records, false)?;
    let options_bytes=serde_json::to_vec(&serde_json::json!({"startTime":requested.map(|r|&r.start_time),"endTime":requested.map(|r|&r.end_time),"maxPages":max_pages,"timeoutSeconds":timeout})).map_err(|_|anyhow!("grok_history_aside_control_invalid"))?;
    write_options(
        &mut guard,
        &options,
        &options_bytes,
        Instant::now() + Duration::from_secs(30),
    )?;
    let bytes = frame(
        &mut reader,
        || {
            guard.poll()?;
            Ok(guard.done)
        },
        deadline,
    )?;
    loop {
        guard.poll()?;
        if let Some(status) = guard
            .child
            .try_wait()
            .map_err(|_| anyhow!("grok_history_aside_unavailable"))?
        {
            if !status.success() || !guard.done {
                bail!("grok_history_cleanup_incomplete");
            }
            break;
        }
        if Instant::now() >= deadline {
            bail!("grok_history_cleanup_incomplete");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(bytes)
}
pub(super) fn collect(requested: Option<&Range>, max_pages: u32, timeout: u32) -> Result<Vec<u8>> {
    execute(PROGRAM, requested, max_pages, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guard_control_preserves_only_valid_diagnostic_fields() {
        let payload=serde_json::json!({"ok":false,"error":"http_server","diagnostic":{"http_status":503,"phase":"walk","successful_pages":2,"elapsed_ms":1250}}).to_string();
        let script = format!("print({})", serde_json::to_string(&payload).unwrap());
        let mut child = Command::new("python3")
            .args(["-B", "-c", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        nonblocking(output.as_raw_fd()).unwrap();
        let mut guard = Guard {
            child,
            output,
            buffer: vec![],
            lines: vec![],
            pipes: vec![],
            done: false,
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let error = loop {
            match guard.poll() {
                Err(error) => break error,
                Ok(()) => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        assert!(error.to_string()=="grok_history_http_server http_status=503 phase=walk successful_pages=2 elapsed_ms=1250");
    }
    #[test]
    fn safe_failure_diagnostics_reject_private_or_invalid_values() {
        let valid = serde_json::json!({"http_status":503,"phase":"walk","successful_pages":2,"elapsed_ms":1250});
        assert!(control_failure("http_server", Some(&valid)).to_string() == "grok_history_http_server http_status=503 phase=walk successful_pages=2 elapsed_ms=1250");
        let mut cases = vec![];
        for key in ["url", "query", "cursor", "account", "body", "token"] {
            let mut value = valid.clone();
            value[key] = serde_json::json!("private-canary");
            cases.push(value);
        }
        for (key, bad) in [
            ("http_status", serde_json::json!(true)),
            ("http_status", serde_json::json!(600)),
            ("http_status", serde_json::json!("private-canary")),
            ("phase", serde_json::json!("private-canary")),
            ("successful_pages", serde_json::json!(-1)),
            ("successful_pages", serde_json::json!(513)),
            ("elapsed_ms", serde_json::json!(3600001)),
            ("elapsed_ms", serde_json::json!(1.5)),
        ] {
            let mut value = valid.clone();
            value[key] = bad;
            cases.push(value);
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("http_status");
        cases.push(missing);
        for value in cases {
            let message = control_failure("http_error", Some(&value)).to_string();
            assert!(message == "grok_history_http_error");
            assert!(!message.contains("private-canary"));
        }
        assert!(
            control_failure("private-canary", Some(&valid)).to_string()
                == "grok_history_browser_transport_unavailable"
        );
    }
    #[test]
    fn helper_diagnostic_validation_is_synthetic_and_private() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/commands/grok_bot_history/aside.test.py"
        );
        let result = Command::new("python3").args(["-B", path]).output().unwrap();
        assert!(
            result.status.success(),
            "synthetic diagnostic validation failed"
        );
    }
    #[test]
    #[ignore = "opt-in synthetic local Aside reverse FIFO test; no Grok/native API"]
    fn local_aside_reverse_fifo_synthetic() {
        let program="let options=JSON.parse(await fs.readFile('options.pipe','utf8'));globalThis.grokTab=await openTab('about:blank');let supported=await grokTab.evaluate(value=>value.synthetic===true,{synthetic:true});if(!supported)throw new Error('synthetic_evaluate_argument');let body=Buffer.from(JSON.stringify({canary:'synthetic-private-only',valid:options.maxPages===4}),'utf8');let head=Buffer.alloc(4);head.writeUInt32BE(body.length);await fs.writeFile('records.pipe',Buffer.concat([head,body]));body.fill(0);console.log('GROK_STATUS:complete');";
        let bytes = execute(program, None, 4, 20)
            .unwrap_or_else(|_| panic!("synthetic reverse FIFO failed"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["canary"] == "synthetic-private-only" && value["valid"] == true);
    }
    #[test]
    #[ignore = "opt-in local browser runtime capabilities only; about:blank, no Grok API"]
    fn local_aside_browser_runtime_synthetic() {
        let program = r#"let options=JSON.parse(await fs.readFile('options.pipe','utf8'));globalThis.grokTab=await openTab('about:blank');let sync=await grokTab.evaluate(value=>value.synthetic===true,{synthetic:true});let asyncArg=await grokTab.evaluate(async value=>{await Promise.resolve();return value.synthetic===true;},{synthetic:true});let asyncObject=await grokTab.evaluate(async value=>({supported:value.synthetic===true}),{synthetic:true});let asyncString=await grokTab.evaluate('(async()=>{await Promise.resolve();return true;})()');let features=await grokTab.evaluate(()=>({bigint:typeof BigInt==='function',text_decoder:typeof TextDecoder==='function',abort_timeout:typeof AbortSignal.timeout==='function',object_has_own:typeof Object.hasOwn==='function',stream_reader:typeof ReadableStream==='function'}));let body=Buffer.from(JSON.stringify({sync:sync===true,async_argument:asyncArg===true,async_object:asyncObject?.supported===true,async_string:asyncString===true,...features}),'utf8');let head=Buffer.alloc(4);head.writeUInt32BE(body.length);await fs.writeFile('records.pipe',Buffer.concat([head,body]));body.fill(0);console.log('GROK_STATUS:complete');"#;
        let bytes = execute(program, None, 4, 20)
            .unwrap_or_else(|_| panic!("synthetic runtime check failed"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for key in [
            "sync",
            "async_argument",
            "async_object",
            "async_string",
            "bigint",
            "text_decoder",
            "abort_timeout",
            "object_has_own",
            "stream_reader",
        ] {
            println!(
                "synthetic_capability={key} available={}",
                value[key] == true
            );
        }
        assert!(value.as_object().unwrap().values().all(|v| v == true));
    }
    #[test]
    #[ignore = "opt-in real local browser/collector/FIFO synthetic pipeline; no website requests"]
    fn local_aside_collector_pipeline_synthetic() {
        run_local_aside_pipeline(false);
    }
    #[test]
    #[ignore = "opt-in delayed local browser pipeline over 120 seconds; no website requests"]
    fn local_aside_collector_delayed_synthetic() {
        run_local_aside_pipeline(true);
    }
    fn run_local_aside_pipeline(delayed_only: bool) {
        let setup = r##"globalThis.grokTab = await openTab('about:blank');
await grokTab.evaluate(()=>{document.body.innerHTML='<a href="#">Usage</a>';const url='https://grok.com/dashboard/api/dashboard/v2/query-billing-items?scope=user&startTime=2000-01-01T00%3A00%3A00Z&endTime=2000-02-01T00%3A00%3A00Z&pageSize=20';window.fetch=()=>Promise.resolve(new Response(JSON.stringify({data:[{billingItemId:'synthetic-item',eventTime:'2000-01-02T00:00:00.000Z',product:'grok-bot',chargeCategory:'included',usageType:'included',principal:{type:'user',id:'synthetic-owner'},hasTokenMeters:true,totalTokens:'1',usage:{'prompt-text-tokens':'1','completion-text-tokens':'0','cached-prompt-text-tokens':'0','cache-write-text-tokens':'0'}}],pagination:{hasMore:false}}),{status:200,headers:{'content-type':'application/json'}}));document.querySelector('a').addEventListener('click',()=>{void window.fetch(new URL(url));});Object.defineProperty(performance,'getEntriesByType',{value:()=>[{name:url}]});});"##;
        let diagnostic = r#"let diagnostic={syntheticFailure:true,stage,code:code||fallback,type:['Error','TypeError','ReferenceError','RangeError'].includes(error?.name)?error.name:'other',mentions_undefined:message.includes('undefined'),mentions_not_function:message.includes('not a function'),mentions_not_defined:message.includes('not defined'),mentions_state:message.includes('__grokHistory'),mentions_baseline:message.includes('baseline'),mentions_options:message.includes('options')};let data=Buffer.from(JSON.stringify(diagnostic),'utf8');let head=Buffer.alloc(4);head.writeUInt32BE(data.length);await fs.writeFile('records.pipe',Buffer.concat([head,data]));console.log('GROK_STATUS:complete');"#;

        let large_setup = r##"globalThis.grokTab = await openTab('about:blank');
await grokTab.evaluate(()=>{document.body.innerHTML='<a href="#">Usage</a>';const url='https://grok.com/dashboard/api/dashboard/v2/query-billing-items?scope=user&startTime=2000-01-01T00%3A00%3A00Z&endTime=2000-02-01T00%3A00%3A00Z&pageSize=20';window.fetch=raw=>{const page=Number(new URL(String(raw)).searchParams.get('cursor')||'0');const data=Array.from({length:100},(_,i)=>({billingItemId:'synthetic-'+page+'-'+i+'-'+String.fromCodePoint(0x1f9ea).repeat(80),eventTime:'2000-01-02T00:00:00.000Z',product:'grok-bot',chargeCategory:'included',usageType:'included',principal:{type:'user',id:'synthetic-owner'},hasTokenMeters:true,totalTokens:'1',usage:{'prompt-text-tokens':'1','completion-text-tokens':'0','cached-prompt-text-tokens':'0','cache-write-text-tokens':'0'}}));return Promise.resolve(new Response(JSON.stringify({data,pagination:page<19?{hasMore:true,nextCursor:String(page+1)}:{hasMore:false}}),{status:200,headers:{'content-type':'application/json'}}));};document.querySelector('a').addEventListener('click',()=>{void window.fetch(new URL(url));});Object.defineProperty(performance,'getEntriesByType',{value:()=>[{name:url}]});});"##;
        // The aggregate wait exceeds the observed single-evaluation deadline,
        // while each page resolves before the bounded per-request deadline.
        let slow_setup = large_setup.replace("page<19", "page<2").replace(
            "return Promise.resolve(new Response",
            "await new Promise(resolve=>setTimeout(resolve,35000));return Promise.resolve(new Response",
        ).replace("window.fetch=raw=>{", "window.fetch=async raw=>{");
        for (setup, expected, delayed) in [
            (setup, 1, false),
            (large_setup, 2000, false),
            (slow_setup.as_str(), 300, true),
        ] {
            if delayed != delayed_only {
                continue;
            }
            let started = Instant::now();
            let program = PROGRAM
                .replace(
                    "globalThis.grokTab = await openTab('https://grok.com/dashboard');",
                    setup,
                )
                .replace(
                    "location.origin !== 'https://grok.com'",
                    "location.origin !== 'null'",
                )
                .replace(
                    "url.origin===location.origin&&url.pathname===path",
                    "url.origin==='https://grok.com'&&url.pathname===path",
                )
                .replace(
                    "console.log('GROK_STATUS:error:'+(code||fallback));",
                    diagnostic,
                );
            let bytes = execute(&program, None, 128, 180)
                .unwrap_or_else(|_| panic!("synthetic local pipeline transport failed"));
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if value.get("syntheticFailure") == Some(&serde_json::Value::Bool(true)) {
                for key in [
                    "stage",
                    "code",
                    "type",
                    "mentions_undefined",
                    "mentions_not_function",
                    "mentions_not_defined",
                    "mentions_state",
                    "mentions_baseline",
                    "mentions_options",
                ] {
                    println!("synthetic_pipeline_{key}={}", value[key]);
                }
                panic!("synthetic browser pipeline stopped");
            }
            let snapshot =
                decode(&bytes).unwrap_or_else(|_| panic!("synthetic source packet invalid"));
            assert!(snapshot.owner == "synthetic-owner" && snapshot.records.len() == expected);
            if delayed {
                assert!(started.elapsed() >= Duration::from_secs(120));
            }
        }
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn fifo_paths_reject_owner_mode_symlink_type_and_inode_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("records.pipe");
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        let info = fs::symlink_metadata(&path).unwrap();
        assert!(check_fifo_as(&info, info.dev(), info.ino(), info.uid().wrapping_add(1)).is_err());
        assert!(check_fifo(&info, info.dev(), info.ino().wrapping_add(1)).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(check_fifo(
            &fs::symlink_metadata(&path).unwrap(),
            info.dev(),
            info.ino()
        )
        .is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(open_pipe(&(alias, info.dev(), info.ino()), false).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"private-canary").unwrap();
        assert!(open_pipe(&(path, info.dev(), info.ino()), false).is_err());
    }
    fn pipe() -> (File, File) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let reader = unsafe { File::from_raw_fd(fds[0]) };
        let writer = unsafe { File::from_raw_fd(fds[1]) };
        nonblocking(reader.as_raw_fd()).unwrap();
        (reader, writer)
    }
    #[test]
    fn frame_limits_eof_and_deadline_use_real_private_pipes() {
        let payload = b"synthetic-canary";
        for variant in 0..5 {
            let (mut reader, mut writer) = pipe();
            let mut bytes = Vec::new();
            match variant {
                0 => {
                    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(payload);
                }
                1 => bytes.extend_from_slice(&((LIMIT + 1) as u32).to_be_bytes()),
                2 => bytes.extend_from_slice(&[0, 0, 0]),
                3 => {
                    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(payload);
                    bytes.push(1);
                }
                _ => bytes.extend_from_slice(&100u32.to_be_bytes()),
            }
            writer.write_all(&bytes).unwrap();
            drop(writer);
            let result = frame(
                &mut reader,
                || Ok(false),
                Instant::now() + Duration::from_secs(1),
            );
            if variant == 0 {
                assert!(result.unwrap() == payload);
            } else {
                assert!(result.err().unwrap().to_string() == "grok_history_private_frame_invalid");
            }
        }
        let (mut reader, _writer) = pipe();
        assert!(
            frame(
                &mut reader,
                || Ok(false),
                Instant::now() + Duration::from_millis(10)
            )
            .err()
            .unwrap()
            .to_string()
                == "grok_history_private_channel_deadline"
        );
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;
    #[test]
    fn child_control_and_stderr_cannot_inject_private_diagnostics() {
        for output in [
            r#"{"ok":false,"error":"private-canary-value"}"#,
            "private-canary-malformed",
        ] {
            let script =
                format!("printf '%s' 'private-canary-stderr' >&2\nprintf '%s\\n' '{output}'");
            let mut child = Command::new("/bin/sh")
                .args(["-c", &script])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            nonblocking(stdout.as_raw_fd()).unwrap();
            let mut guard = Guard {
                child,
                output: stdout,
                buffer: vec![],
                lines: vec![],
                pipes: vec![],
                done: false,
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            let error = loop {
                match guard.poll() {
                    Err(error) => break error,
                    Ok(()) => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            };
            assert!(!format!("{error:?}").contains("private-canary"));
        }
    }
}
