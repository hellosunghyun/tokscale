'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {instant,plan,diagnostic}=require('./grok-bot-backfill.cjs');
const start='2026-01-01T00:00:00Z',end='2026-01-04T00:00:00Z';
test('half-open UTC days are adjacent without overlap and resume only complete same-account ranges',()=>{
 const prior={owner:'synthetic-a',ownerRechecked:true,apiPaginationComplete:true,range:{startTime:'2026-01-02T00:00:00Z',endTime:'2026-01-03T00:00:00Z'}};
 const days=plan(start,end,[prior],'synthetic-a',7);
 assert.deepEqual(days.map(d=>d.cached),[false,true,false]);
 assert.equal(days.length,3);
 for(const day of days)assert.equal(instant(day.range.endTime)-instant(day.range.startTime),86400000);
 assert.equal(days[0].range.startTime,days[1].range.endTime);
 assert.equal(plan(start,end,[{...prior,owner:'synthetic-b'}],'synthetic-a',7).some(d=>d.cached),false);
 assert.equal(plan(start,end,[{...prior,apiPaginationComplete:false}],'synthetic-a',7).some(d=>d.cached),false);
 assert.equal(plan(start,end,[{...prior,ownerRechecked:false}],'synthetic-a',7).some(d=>d.cached),false);
});
test('invalid calendar days, timezones and over-budget plans fail before any collection',()=>{
 for(const value of ['2026-02-30T00:00:00Z','2026-01-01T01:00:00Z','2026-01-01T00:00:00+00:00','private-canary'])assert.throws(()=>instant(value));
 assert.throws(()=>plan(end,start,[],'a',7));
 assert.throws(()=>plan(start,end,[],'a',2));
 assert.throws(()=>plan(start,end,[],'a',32));
});
test('error output preserves only safe fixed fields',()=>{
 for(const [code,status] of [['http_auth',401],['http_permission',403],['http_rate_limit',429],['http_server',503]]) {
 const out=diagnostic(`private-canary grok_history_${code} http_status=${status} phase=walk successful_pages=2 elapsed_ms=300\n`);
 assert.deepEqual(out,{failure:code,diagnostic:{http_status:status,phase:'walk',successful_pages:2,elapsed_ms:300}});
 assert.equal(JSON.stringify(out).includes('private-canary'),false);
 }
 assert.deepEqual(diagnostic('grok_history_private_canary\n'),{failure:'collection_failed'});
 assert.deepEqual(diagnostic('grok_history_http_error http_status=999 phase=walk successful_pages=2 elapsed_ms=300\n'),{failure:'http_error'});
});
test('partial batch preserves completed checkpoints and resume skips them without retrying inside a run',()=>{
 const fs=require('node:fs'),os=require('node:os'),path=require('node:path');
 const {main}=require('./grok-bot-backfill.cjs');
 const dir=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'tokscale-backfill-synthetic-')));
 fs.chmodSync(dir,0o700);
 const file=path.join(dir,'history.json');
 const snapshot=(start,end,id)=>({owner:'synthetic-owner',ownerRechecked:true,apiPaginationComplete:true,range:{startTime:start,endTime:end},records:[{billingItemId:id,eventTime:start,product:'grok-bot',principal:{id:'synthetic-owner'}}]});
 fs.writeFileSync(file,JSON.stringify({version:1,snapshots:[snapshot('2026-01-03T00:00:00Z',end,'synthetic-existing')]}),{mode:0o600});
 let syncs=0,fail=true;const events=[];
 const runner=(_binary,_dir,args)=>{
  const sync=args.includes('sync');
  const metadata={source:'grok-dashboard-billing-v2',transport:'aside-local',api_pagination_complete:true,coverage:'requested_range_only',cache_saved:sync,cache_read_ok:!sync};
  if(sync){syncs++;const a=args[args.indexOf('--start')+1],b=args[args.indexOf('--end')+1];
   if(a===start&&fail)return{status:1,stdout:'',stderr:'grok_history_http_rate_limit http_status=429 phase=walk successful_pages=1 elapsed_ms=3\n'};
   const value=JSON.parse(fs.readFileSync(file));value.snapshots.push(snapshot(a,b,'synthetic-'+a));fs.writeFileSync(file,JSON.stringify(value));
  }
  return{status:0,stdout:JSON.stringify(metadata),stderr:''};
 };
 const args=['--binary',process.execPath,'--data-dir',dir,'--start',start,'--end',end,'--max-days','3'];
 const log=console.log;console.log=value=>events.push(JSON.parse(value));
 try{
  assert.equal(main(args,runner),1);
  assert.equal(syncs,2);assert.equal(JSON.parse(fs.readFileSync(file)).snapshots.length,2);
  assert.equal(events.at(-1).failure,'http_rate_limit');assert.equal(events.at(-1).cache_preserved,true);
  fail=false;events.length=0;syncs=0;
  assert.equal(main(args,runner),0);assert.equal(syncs,1);
  assert.equal(events.at(-1).skipped_complete_days,2);assert.equal(events.at(-1).complete,true);
 }finally{console.log=log;fs.rmSync(dir,{recursive:true,force:true});}
});
