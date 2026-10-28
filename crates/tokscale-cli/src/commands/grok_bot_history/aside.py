"""Bundled local-only Aside REPL controller. Source data uses FIFOs, never output."""
import json
import os
import pty
import re
import select
import signal
import stat
import subprocess
import sys
import time

class Stopped(Exception):
    pass

def fifo_stat(path):
    info = os.lstat(path)
    if not stat.S_ISFIFO(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise Stopped('private_channel_invalid')
    return info

def drain_exit(proc, fd, timeout):
    end = time.monotonic() + timeout
    while proc.poll() is None and time.monotonic() < end:
        ready, _, _ = select.select([fd], [], [], .05)
        if ready:
            try:
                os.read(fd, 65536)
            except OSError:
                break
    return proc.wait(timeout=max(.1, end-time.monotonic()))

def exchange(fd, code=None, timeout=25):
    data = (code + '\r').encode() if code is not None else b''
    offset = 0
    captured = b''
    end = time.monotonic()+timeout
    os.set_blocking(fd, False)
    try:
        while time.monotonic() < end:
            reads, writes, _ = select.select([fd], [fd] if offset < len(data) else [], [], .05)
            if writes:
                try:
                    offset += os.write(fd, data[offset:offset+256])
                except BlockingIOError:
                    pass
            if reads:
                try:
                    chunk = os.read(fd, 65536)
                except BlockingIOError:
                    continue
                if not chunk:
                    raise Stopped('aside_closed')
                captured += chunk
                if len(captured) > 2*1024*1024:
                    raise Stopped('aside_output_limit')
                clean = re.sub(rb'\x1b\[[0-9;]*[A-Za-z]', b'', captured)
                last = max(clean.rfind(b'[ok |'), clean.rfind(b'[error |'))
                if offset == len(data) and b'repl >' in clean and (code is None or last >= 0 and clean.rfind(b'repl >') > last):
                    return clean
        raise Stopped('aside_deadline')
    finally:
        os.set_blocking(fd, True)

def run_program(fd, source, timeout):
    exchange(fd, "globalThis.grokProgram='';void 0;")
    for offset in range(0, len(source), 350):
        exchange(fd, 'globalThis.grokProgram+='+json.dumps(source[offset:offset+350])+';void 0;')
    output = exchange(fd, "await (0,eval)('(async()=>{'+globalThis.grokProgram+'})()');globalThis.grokProgram=null;console.log('GROK_DRIVER:'+(globalThis.__grokCollectionSteps?'ready':'single'));void 0;", min(115, timeout))
    if b'[error |' in output or re.search(rb'(?:^|[\r\n])GROK_DRIVER:ready(?:[\r\n]|$)', output) is None:
        return output
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        output = exchange(fd, "globalThis.grokDriverResult=await globalThis.__grokCollectionSteps.next();console.log('GROK_STEP:'+(globalThis.grokDriverResult.done?'done':'pending'));globalThis.grokDriverResult=null;void 0;", min(35, max(1, deadline-time.monotonic())))
        if b'[error |' in output or re.search(rb'(?:^|[\r\n])GROK_STEP:done(?:[\r\n]|$)', output):
            return output
        if re.search(rb'(?:^|[\r\n])GROK_STEP:pending(?:[\r\n]|$)', output) is None:
            raise Stopped('aside_control_invalid')
    raise Stopped('collection_deadline')

def safe_diagnostic(value):
    keys = {'http_status', 'phase', 'successful_pages', 'elapsed_ms'}
    if not isinstance(value, dict) or set(value) != keys:
        return None
    status = value['http_status']
    if status is not None and (type(status) is not int or not 100 <= status <= 599):
        return None
    if value['phase'] not in ('discover', 'owner', 'walk', 'recheck'):
        return None
    if type(value['successful_pages']) is not int or not 0 <= value['successful_pages'] <= 512:
        return None
    if type(value['elapsed_ms']) is not int or not 0 <= value['elapsed_ms'] <= 3600000:
        return None
    return {key: value[key] for key in ('http_status', 'phase', 'successful_pages', 'elapsed_ms')}

def parse_diagnostic_output(output):
    found = re.findall(rb'(?:^|[\r\n])GROK_DIAGNOSTIC:(\{[^\r\n]*\})(?=[\r\n]|$)', output)
    if len(found) != 1 or len(found[0]) > 1024:
        return None
    try:
        return safe_diagnostic(json.loads(found[0]))
    except (ValueError, TypeError):
        return None

def main():
    fd = None
    proc = None
    pipes = []
    ok = False
    failure = 'aside_unavailable'
    diagnostic = None
    closed = False
    try:
        config = json.loads(sys.stdin.buffer.readline(262145))
        source = config['program']
        budget = config.get('timeoutSeconds', 100)
        if not isinstance(budget,int) or isinstance(budget,bool) or not 10 <= budget <= 600:
            raise Stopped('aside_control_invalid')
        if not isinstance(source, str) or len(source) > 240000:
            raise Stopped('aside_control_invalid')
        fd, slave = pty.openpty()
        try:
            proc = subprocess.Popen(['aside', 'repl', '--host', 'local'], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
        finally:
            os.close(slave)
        startup = exchange(fd)
        match = re.search(rb'sessionDir: ([^\r\n]+)', startup)
        if not match:
            raise Stopped('aside_session_unavailable')
        root = os.fsdecode(match[1])
        info = os.lstat(root)
        if os.path.realpath(root) != os.path.abspath(root) or not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o022:
            raise Stopped('private_channel_invalid')
        hello = {'version': 1, 'root': root, 'pipes': []}
        for name in ['options.pipe', 'records.pipe']:
            path = os.path.join(root, name)
            os.mkfifo(path, 0o600)
            info = fifo_stat(path)
            pipes.append((path, info.st_dev, info.st_ino))
            hello['pipes'].append({'name': name, 'device': info.st_dev, 'inode': info.st_ino})
        # This stdout is a private Rust control pipe. No source records go here.
        print(json.dumps(hello), flush=True)
        startup = None
        output = run_program(fd, source, budget+40)
        ok = re.search(rb'(?:^|[\r\n])GROK_STATUS:complete(?:[\r\n]|$)', output) is not None and b'[error |' not in output
        if not ok:
            found = re.findall(rb'GROK_STATUS:error:([a-z_]+)', output)
            allowed = {'owner_unavailable','owner_changed','browser_dashboard_unavailable','baseline_unavailable','baseline_scope_invalid','invalid_range','invalid_page','invalid_cursor','cursor_cycle','duplicate_item','scope_drift','invalid_bot_record','charge_category_unavailable','usage_type_unavailable','token_meters_unavailable','meter_schema_unavailable','browser_runtime_unsupported','browser_range_runtime','browser_owner_runtime','browser_walk_runtime','browser_recheck_runtime','channel_encoding_unavailable','invalid_meter','meter_overflow','total_mismatch','event_outside_range','page_limit','byte_limit','collection_deadline','page_request_deadline','http_transport','collection_deadline_discover','collection_deadline_owner','collection_deadline_walk','collection_deadline_recheck','http_auth','http_permission','http_rate_limit','http_error','http_server','response_not_json','response_schema','private_channel_invalid','cleanup_incomplete'}
            candidate = found[-1].decode() if found else ''
            failure = candidate if candidate in allowed else 'browser_collection_unavailable'
            diagnostic = parse_diagnostic_output(output)
        output = None
    except Stopped as error:
        failure = str(error)
    except Exception:
        failure = 'aside_unavailable'
    finally:
        if proc and proc.poll() is None:
            try:
                # Also runs if the main program timed out before its own finally.
                cleanup = "if(globalThis.grokTab){try{await grokTab.evaluate(()=>{if(globalThis.__grokHistory){globalThis.__grokHistory.restore();delete globalThis.__grokHistory;}delete globalThis.__grokHistoryPayload;delete globalThis.__grokHistoryIterator;delete globalThis.__grokHistoryPending;});}catch{}await closeTab(grokTab);globalThis.grokTab=null;}globalThis.grokProgram=null;globalThis.__grokCollectionSteps=null;globalThis.grokDriverResult=null;console.log('GROK_CLEANUP:complete');void 0;"
                cleanup_output = exchange(fd, cleanup)
                if re.search(rb'(?:^|[\r\n])GROK_CLEANUP:complete(?:[\r\n]|$)', cleanup_output) is None or b'[error |' in cleanup_output:
                    raise Stopped('cleanup_incomplete')
                os.write(fd, b'exit\r')
                closed = drain_exit(proc, fd, 20) == 0
            except Exception:
                proc.terminate()
                try:
                    drain_exit(proc, fd, 3)
                except Exception:
                    proc.kill()
                    proc.wait()
        for path, device, inode in pipes:
            try:
                current = fifo_stat(path)
                if (current.st_dev, current.st_ino) != (device, inode):
                    raise Stopped('cleanup_incomplete')
                os.unlink(path)
            except FileNotFoundError:
                pass
            except (OSError, Stopped):
                ok = False
                failure = 'cleanup_incomplete'
        if fd is not None:
            os.close(fd)
    if not closed:
        ok = False
        failure = 'cleanup_incomplete'
    print(json.dumps({'ok': ok, 'error': None if ok else failure, 'diagnostic': diagnostic if not ok else None}), flush=True)
    return 0 if ok else 1

if __name__ == '__main__':
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(Stopped('aside_cancelled')))
    sys.exit(main())
