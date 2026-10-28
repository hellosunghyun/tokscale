#!/usr/bin/env node
'use strict';
// Local orchestration only. Credentials stay in the existing browser transport.
const fs = require('node:fs');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const DAY = 86400000;
function instant(value) {
  if (typeof value !== 'string' || !/^\d{4}-\d\d-\d\dT00:00:00Z$/.test(value)) throw Error('invalid_utc_day');
  const time = Date.parse(value);
  if (!Number.isSafeInteger(time) || new Date(time).toISOString().replace('.000Z', 'Z') !== value) throw Error('invalid_utc_day');
  return time;
}
function stamp(time) { return new Date(time).toISOString().replace('.000Z', 'Z'); }
function plan(start, end, snapshots, owner, maxDays) {
  const first = instant(start), last = instant(end);
  if (last <= first || !Number.isInteger(maxDays) || maxDays < 1 || maxDays > 31 || (last-first)/DAY > maxDays) throw Error('range_budget_exceeded');
  const days = [];
  for (let stop = last; stop > first; stop -= DAY) {
    const range = {startTime: stamp(stop-DAY), endTime: stamp(stop)};
    const cached = snapshots.some(s => s.owner === owner && s.ownerRechecked === true && s.apiPaginationComplete === true && s.range.startTime === range.startTime && s.range.endTime === range.endTime);
    days.push({range, cached});
  }
  return days;
}
function diagnostic(text) {
  const match = /grok_history_([a-z_]+)(?: http_status=(none|\d{3}) phase=(discover|owner|walk|recheck) successful_pages=(\d+) elapsed_ms=(\d+))?(?:\s|$)/.exec(text);
  const codes = new Set(['http_auth','http_permission','http_rate_limit','http_server','http_error','http_transport','page_limit','byte_limit','collection_deadline','page_request_deadline','collection_deadline_discover','collection_deadline_owner','collection_deadline_walk','collection_deadline_recheck','owner_changed','owner_unavailable','browser_dashboard_unavailable','baseline_unavailable','invalid_page','invalid_cursor','cursor_cycle','duplicate_item','scope_drift','event_outside_range','response_schema','response_not_json','private_channel_deadline','browser_transport_unavailable']);
  const out = {failure: match && codes.has(match[1]) ? match[1] : 'collection_failed'};
  if (match?.[2] && (match[2] === 'none' || +match[2] >= 100 && +match[2] <= 599) && +match[4] <= 512 && +match[5] <= 3600000) out.diagnostic = {http_status: match[2] === 'none' ? null : +match[2], phase: match[3], successful_pages: +match[4], elapsed_ms: +match[5]};
  return out;
}
function safeDirectory(dir) {
  if (!path.isAbsolute(dir) || path.resolve(dir) !== dir) throw Error('private_directory_invalid');
  for (let p = dir;; p = path.dirname(p)) {
    if (fs.lstatSync(p).isSymbolicLink() || fs.existsSync(path.join(p,'.git'))) throw Error('private_directory_invalid');
    if (p === path.dirname(p)) break;
  }
  const info = fs.statSync(dir);
  if (!info.isDirectory() || info.uid !== process.getuid() || (info.mode & 0o7777) !== 0o700) throw Error('private_directory_invalid');
}
function store(dir) {
  const file = path.join(dir,'history.json'), info = fs.lstatSync(file);
  if (!info.isFile() || info.nlink !== 1 || info.uid !== process.getuid() || (info.mode & 0o7777) !== 0o600 || info.size > 64*1024*1024) throw Error('private_cache_invalid');
  const bytes = fs.readFileSync(file), value = JSON.parse(bytes);
  if (value.version !== 1 || !Array.isArray(value.snapshots) || !value.snapshots.length) throw Error('private_cache_invalid');
  return {bytes, snapshots:value.snapshots};
}
function invoke(binary, dir, args) {
  return spawnSync(binary, ['--no-spinner', ...args, '--redacted'], {env:{...process.env,TOKSCALE_GROK_BOT_DATA_DIR:dir},encoding:'utf8',timeout:400000,maxBuffer:1024*1024,stdio:['ignore','pipe','pipe']});
}
function metadata(result, saved) {
  if (result.error || result.status !== 0) return false;
  try { const v=JSON.parse(result.stdout); return v.source === 'grok-dashboard-billing-v2' && v.transport === 'aside-local' && v.api_pagination_complete === true && v.coverage === 'requested_range_only' && v.cache_saved === saved && v.cache_read_ok === !saved; } catch { return false; }
}
function main(argv, runner = invoke) {
  const args = {};
  for (let i=0;i<argv.length;i+=2) { if (!['--binary','--data-dir','--start','--end','--max-days'].includes(argv[i]) || !argv[i+1] || Object.hasOwn(args,argv[i])) throw Error('invalid_arguments'); args[argv[i]]=argv[i+1]; }
  const binary=args['--binary'], dir=args['--data-dir'];
  if (!binary || !path.isAbsolute(binary) || !fs.statSync(binary).isFile()) throw Error('binary_unavailable');
  safeDirectory(dir);
  const initial=store(dir), owner=initial.snapshots.at(-1).owner;
  if (typeof owner !== 'string' || !owner || initial.snapshots.some(s=>s.owner!==owner)) throw Error('account_selection_required');
  if (!metadata(runner(binary,dir,['--offline','grok-bot','history']),false)) throw Error('initial_cache_invalid');
  const days=plan(args['--start'],args['--end'],initial.snapshots,owner,Number(args['--max-days'] || 7));
  const summary={requested_days:days.length,skipped_complete_days:0,collected_days:0,empty_days:0,records_in_new_days:0,complete:false,retention_known:false};
  for (const day of days) {
    if(day.cached){summary.skipped_complete_days++;continue;}
    const before=store(dir), started=Date.now();
    const result=runner(binary,dir,['grok-bot','sync','--transport','aside','--start',day.range.startTime,'--end',day.range.endTime]);
    const after=store(dir);
    if(!metadata(result,true)) { console.log(JSON.stringify({event:'stopped',...diagnostic(result.stderr || ''),cache_preserved:before.bytes.equals(after.bytes),elapsed_seconds:Math.round((Date.now()-started)/1000),...summary})); return 1; }
    const latest=after.snapshots.at(-1);
    if(latest.owner!==owner || latest.range.startTime!==day.range.startTime || latest.range.endTime!==day.range.endTime || !latest.apiPaginationComplete || !latest.ownerRechecked) throw Error('account_or_range_changed');
    const ids=new Set();
    for(const row of latest.records){if(row.product!=='grok-bot'||row.principal.id!==owner||ids.has(row.billingItemId)||Date.parse(row.eventTime)<instant(day.range.startTime)||Date.parse(row.eventTime)>=instant(day.range.endTime))throw Error('record_validation_failed');ids.add(row.billingItemId);}
    if(!metadata(runner(binary,dir,['--offline','grok-bot','history']),false)) throw Error('readback_failed');
    summary.collected_days++;summary.empty_days+=latest.records.length===0?1:0;summary.records_in_new_days+=latest.records.length;
    console.log(JSON.stringify({event:'day_saved_and_read',elapsed_seconds:Math.round((Date.now()-started)/1000),...summary}));
  }
  summary.complete=true;
  console.log(JSON.stringify({event:'requested_range_complete',...summary}));return 0;
}
module.exports={instant,plan,diagnostic,main};
if(require.main===module){try{process.exitCode=main(process.argv.slice(2));}catch(error){const allowed=new Set(['invalid_utc_day','range_budget_exceeded','invalid_arguments','binary_unavailable','private_directory_invalid','private_cache_invalid','account_selection_required','initial_cache_invalid','account_or_range_changed','record_validation_failed','readback_failed']);console.log(JSON.stringify({event:'stopped',failure:allowed.has(error.message)?error.message:'local_controller_failed'}));process.exitCode=1;}}
