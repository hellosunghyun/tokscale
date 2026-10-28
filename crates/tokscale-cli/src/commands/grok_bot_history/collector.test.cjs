// Synthetic connected execution of the bundled REPL/browser collector, no network.
const vm=require('node:vm'),fs=require('node:fs'),assert=require('node:assert/strict');
const source=fs.readFileSync(__dirname+'/collector.js','utf8');
const baseline='https://grok.com/dashboard/api/dashboard/v2/query-billing-items?scope=user&startTime=2020-01-01T00%3A00%3A00Z&endTime=2020-02-01T00%3A00%3A00Z&pageSize=20';
const bot=(id='synthetic-a')=>({billingItemId:id,eventTime:'2020-01-02T00:00:00.123Z',product:'grok-bot',chargeCategory:'included',usageType:'included',principal:{type:'user',id:'private-canary-owner'},hasTokenMeters:true,totalTokens:'9007199254741002',usage:{'prompt-text-tokens':'9007199254740993','completion-text-tokens':'2','cached-prompt-text-tokens':'3','cache-write-text-tokens':'4'}});
async function run(scenario,extra={}){
 const calls=[],logs=[];let frame,closed=false,lastPromise;
 const first={data:scenario==='leading_other'?[{billingItemId:'leading-other',product:'grok-chat'}]:scenario==='empty_owner'?[]:[bot()],pagination:{hasMore:true,nextCursor:'private-canary-next'}};
 if(scenario==='empty_owner')first.pagination={hasMore:false};
 function fetch(url,init){
  const u=new URL(url);if(!init){lastPromise=Promise.resolve(new Response(JSON.stringify(first),{status:200,headers:{'content-type':'application/json'}}));return lastPromise;}calls.push({url:String(url),init});let value,status=200,mime='application/json';
  if(calls.length===1)value=first;
  else if(!u.searchParams.has('cursor')){value=JSON.parse(JSON.stringify(first));if(scenario==='switch')value.data[0].principal.id='private-canary-other';if(extra.startTime&&u.searchParams.get('startTime')===extra.startTime)value={data:[],pagination:{hasMore:false}};}
  else if(u.searchParams.get('cursor')==='private-canary-next')value={data:[{billingItemId:'synthetic-other',product:'grok-chat',usage:{'Human readable':'not-a-bot-meter'},pricing:null}],pagination:{hasMore:true,nextCursor:'private-canary-final'}};
  else value={data:[bot('synthetic-b')],pagination:{hasMore:false}};
  if(calls.length===2){
   if(scenario==='auth')status=401;if(scenario==='forbidden')status=403;if(scenario==='rate')status=429;if(scenario==='http')status=500;if(scenario==='server')status=503;if(scenario==='client')status=400;if(scenario==='html')mime='text/html';
   if(scenario==='timeout'){lastPromise=Promise.reject(new Error('private-canary-network'));return lastPromise;}
   if(scenario==='oversize')value={padding:'private-canary'.repeat(30000)};
   if(scenario==='missing_array')value={pagination:{hasMore:false}};
   if(scenario==='missing_more')value={data:[]};
   if(scenario==='cursor_missing')value={data:[],pagination:{hasMore:true}};
   if(scenario==='cursor_cycle')value={data:[],pagination:{hasMore:true,nextCursor:'private-canary-next'}};
   if(scenario==='terminal_cursor')value={data:[],pagination:{hasMore:false,nextCursor:'unexpected'}};
   if(scenario==='duplicate')value={data:[bot()],pagination:{hasMore:false}};
   if(scenario==='scope')value={data:[],pagination:{hasMore:false},scope:'team'};
   if(scenario==='range')value={data:[],pagination:{hasMore:false},startTime:'2021-01-01T00:00:00Z'};
   if(scenario==='owner'){value={data:[bot('synthetic-b')],pagination:{hasMore:false}};value.data[0].principal.id='private-canary-other';}
   if(['missing_meter','bad_number','overflow','sum','has_tokens','event','extra_meter'].includes(scenario)){
    const row=bot('synthetic-b');value={data:[row],pagination:{hasMore:false}};
    if(scenario==='missing_meter')delete row.usage['prompt-text-tokens'];
    if(scenario==='extra_meter')row.usage.extra='0';
    if(scenario==='bad_number')row.totalTokens='1.5';
    if(scenario==='overflow')row.totalTokens='340282366920938463463374607431768211456';
    if(scenario==='sum')row.totalTokens='1';
    if(scenario==='has_tokens')row.hasTokenMeters=false;
    if(scenario==='event')row.eventTime='2020-02-01T00:00:00Z';
   }
  }
  lastPromise=Promise.resolve(new Response(JSON.stringify(value),{status,headers:{'content-type':mime}}));return lastPromise;
 }
 const browser=vm.createContext({URL,TextDecoder,TextEncoder,Uint8Array,Promise,AbortSignal,setTimeout,clearTimeout,location:{origin:'https://grok.com',href:'https://grok.com/dashboard'},performance:{getEntriesByType:()=>[{name:baseline}],now:()=>performance.now()}});
 browser.fetch=fetch;vm.runInContext('globalThis.window=globalThis;',browser);
 const page={evaluate:async(fn,arg)=>{try{if(scenario==='unsafe_diag'&&fn.toString().includes('__grokHistoryIterator.next'))return {waiting:false,done:true,value:{ok:false,error:'http_server',diagnostic:{http_status:503,phase:'walk',successful_pages:1,elapsed_ms:2,url:'private-canary-url'}}};const before=calls.length;const value=await vm.runInContext('('+fn.toString()+')',browser)(arg);assert(calls.length-before<=1,'one API page per evaluation');return value;}catch(error){throw new Error('Evaluation failed: '+error.message);}},locator:ref=>{assert.equal(ref,'e1');return {click:async()=>{const promise=browser.fetch(new URL(baseline));assert.strictEqual(promise,lastPromise);await promise;}};}};
 const options={startTime:null,endTime:null,maxPages:20,timeoutSeconds:30,...extra};
 const local=vm.createContext({Buffer,JSON,console:{log:value=>logs.push(value)},fs:{readFile:async()=>JSON.stringify(options),writeFile:async(name,bytes)=>{assert.equal(name,'records.pipe');frame=Buffer.from(bytes);}},openTab:async()=>page,snapshot:async()=>({tree:'e0 link "Overview"\ne1 link "Usage"'}),closeTab:async()=>{closed=true;}});
 await vm.runInContext('(async()=>{'+source+'})()',local);
 const driver=local.__grokCollectionSteps;while(true){const next=await driver.next();if(next.done)break;assert.equal(next.value,true);}
 assert(closed);assert.strictEqual(browser.fetch,fetch);assert(!browser.__grokHistory);assert(!browser.__grokHistoryIterator);assert(!browser.__grokHistoryPending);assert(!logs.join('').includes('private-canary'));
 if(frame){assert.equal(frame.readUInt32BE(0),frame.length-4);return {packet:JSON.parse(frame.subarray(4)),calls,logs};}
 return {calls,logs};
}
(async()=>{
 const complete=await run('valid');assert(complete.packet);assert.equal(complete.packet.records.length,2);assert.equal(complete.calls.length,4);assert(complete.packet.apiPaginationComplete&&complete.packet.ownerRechecked);assert.equal(complete.packet.records[0].usage['prompt-text-tokens'],'9007199254740993');assert.equal(complete.packet.records[0].model,null);assert.equal(complete.packet.records[0].cost,null);
 const first=new URL(complete.calls[0].url);assert.equal(first.searchParams.get('pageSize'),'100');const next=new URL(complete.calls[1].url);assert.equal(next.searchParams.get('cursor'),'private-canary-next');next.searchParams.delete('cursor');assert.equal(next.href,first.href);assert.equal(complete.calls[1].init.redirect,'error');assert.equal(complete.calls[1].init.credentials,'same-origin');
 for(const scenario of ['auth','forbidden','rate','http','server','client','html','timeout','oversize','missing_array','missing_more','cursor_missing','cursor_cycle','terminal_cursor','duplicate','scope','range','owner','missing_meter','extra_meter','bad_number','overflow','sum','has_tokens','event','switch']){
  const result=await run(scenario);assert(!result.packet,scenario);assert(result.logs.some(line=>line.startsWith('GROK_STATUS:error:')),scenario);assert(result.calls.length<=4);
 }
 for(const [scenario,status,code] of [['auth',401,'http_auth'],['forbidden',403,'http_permission'],['rate',429,'http_rate_limit'],['http',500,'http_server'],['server',503,'http_server'],['client',400,'http_error'],['html',200,'response_not_json']]){
  const result=await run(scenario);assert(!result.packet);assert.equal(result.calls.length,2,'no retry or fallback');
  assert(result.logs.includes('GROK_STATUS:error:'+code));
  const lines=result.logs.filter(value=>value.startsWith('GROK_DIAGNOSTIC:'));assert.equal(lines.length,1);
  const d=JSON.parse(lines[0].slice('GROK_DIAGNOSTIC:'.length));
  assert.deepEqual(Object.keys(d).sort(),['elapsed_ms','http_status','phase','successful_pages']);
  assert.equal(d.http_status,status);assert.equal(d.phase,'walk');assert.equal(d.successful_pages,1);assert(Number.isInteger(d.elapsed_ms)&&d.elapsed_ms>=0);
 }
 const unsafe=await run('unsafe_diag');assert(!unsafe.packet);assert(!unsafe.logs.some(value=>value.startsWith('GROK_DIAGNOSTIC:')));
 const leading=await run('leading_other');assert(leading.packet&&leading.packet.records.length===1);assert.equal(leading.calls.length,4);
 const ownerUnavailable=await run('empty_owner');assert(!ownerUnavailable.packet);assert.equal(ownerUnavailable.calls.length,1);assert(ownerUnavailable.logs.includes('GROK_STATUS:error:owner_unavailable'));
 const limited=await run('valid',{maxPages:2});assert(!limited.packet);assert.equal(limited.calls.length,1);assert(limited.logs.includes('GROK_STATUS:error:page_limit'));
 const deadline=await run('valid',{timeoutSeconds:0});assert(!deadline.packet);assert.equal(deadline.calls.length,0);
 const empty=await run('valid',{startTime:'2021-01-01T00:00:00Z',endTime:'2021-02-01T00:00:00Z'});assert(empty.packet&&empty.packet.records.length===0&&empty.packet.ownerRechecked);
 console.log('collector_connected_synthetic_pass=true complete_empty_faults_limits_privacy_cleanup_pass=true');
})().catch(()=>{console.log('collector_connected_synthetic_pass=false');process.exitCode=1;});
