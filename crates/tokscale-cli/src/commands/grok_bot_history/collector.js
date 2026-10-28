globalThis.__grokCollectionSteps=(async function*(){
// Supported local REPL only. Real records leave through the private FIFO.
let options;
let stage='read_options';
let failureDiagnostic=null;
try {
  options=JSON.parse(await fs.readFile('options.pipe','utf8'));
  yield true;
  stage='open_dashboard';
  globalThis.grokTab = await openTab('https://grok.com/dashboard');
  yield true;
  let view=await snapshot(grokTab,{interactive:true});
  let line=view.tree.split('\n').find(line=>/\blink\b/.test(line)&&line.includes('"Usage"'));
  let ref=(line?.match(/\b(?:e\d+|f\d+e\d+)\b/)||[])[0];view=null;line=null;
  if(!ref)throw new Error('browser_dashboard_unavailable');
  stage='usage_click';
  await grokTab.locator(ref).click();ref=null;
  view=await snapshot(grokTab,{interactive:true});view=null;
  stage='collect';
  await grokTab.evaluate(settings=>{
    globalThis.__grokHistoryIterator=(async function*(){
    let browserStage='discover';
    const diagnostic={http_status:null,successful_pages:0,started:performance.now()};
    const validatedPages=new WeakSet();
    const details=()=>({http_status:diagnostic.http_status,phase:browserStage,successful_pages:diagnostic.successful_pages,elapsed_ms:Math.round(performance.now()-diagnostic.started)});
    try {
      if(location.origin !== 'https://grok.com')throw new Error('browser_dashboard_unavailable');
    const path = '/dashboard/api/dashboard/v2/query-billing-items';
    const original = window.fetch;
    const fail = code => { throw new Error(code); };
    const text = value => typeof value === 'string' && value.length>0 && value.length<=512 && value.trim()===value;
    const meters=['prompt-text-tokens','completion-text-tokens','cached-prompt-text-tokens','cache-write-text-tokens'];
    const maximum=(1n<<128n)-1n;
    function integer(value) {
      if(typeof value!=='string'||value.length>39||!/^\d+$/.test(value))fail('invalid_meter');
      const parsed=BigInt(value);if(parsed>maximum)fail('meter_overflow');return parsed;
    }
    function context(raw) {
      const url=new URL(raw,location.href), keys=[...url.searchParams.keys()];
      if(url.origin!=='https://grok.com'||url.pathname!==path||url.hash||url.username||url.password||new Set(keys).size!==keys.length||keys.some(key=>!['scope','startTime','endTime','pageSize','cursor'].includes(key))||url.searchParams.get('scope')!=='user'||!['20','100'].includes(url.searchParams.get('pageSize')))fail('baseline_scope_invalid');
      const start=url.searchParams.get('startTime'),end=url.searchParams.get('endTime');
      if(!/^\d{4}-\d\d-\d\dT.*Z$/.test(start||'')||!/^\d{4}-\d\d-\d\dT.*Z$/.test(end||'')||!Number.isSafeInteger(Date.parse(start))||!Number.isSafeInteger(Date.parse(end))||Date.parse(start)>=Date.parse(end))fail('invalid_range');
      return url;
    }
    async function read(response, deadline, byteLimit=262144) {
      diagnostic.http_status=Number.isInteger(response.status)&&response.status>=100&&response.status<=599?response.status:null;
      if(response.status===401)fail('http_auth');
      if(response.status===403)fail('http_permission');
      if(response.status===429)fail('http_rate_limit');
      if(response.status>=500&&response.status<=599)fail('http_server');
      if(response.status!==200)fail('http_error');
      if((response.headers.get('content-type')||'').split(';')[0].trim()!=='application/json')fail('response_not_json');
      const reader=response.body?.getReader();if(!reader)fail('response_schema');
      const chunks=[];let size=0;
      try {
        while(true){let timer;let part;try{part=await Promise.race([reader.read(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('collection_deadline')),Math.max(1,deadline-Date.now()));})]);}finally{clearTimeout(timer);}if(part.done)break;size+=part.value.length;if(size>byteLimit)fail('byte_limit');chunks.push(part.value);}
        const bytes=new Uint8Array(size);let offset=0;for(const part of chunks){bytes.set(part,offset);offset+=part.length;}
        let value;try{value=JSON.parse(new TextDecoder().decode(bytes));}catch{fail('response_schema');}finally{bytes.fill(0);}return {value,size};
      } catch(error) {void reader.cancel().catch(()=>{});throw error;}
    }
    function page(value,url) {
      if(!value||typeof value!=='object'||!Array.isArray(value.data)||!value.pagination||typeof value.pagination.hasMore!=='boolean')fail('invalid_page');
      for(const key of ['scope','startTime','endTime'])if(Object.hasOwn(value,key)&&value[key]!==url.searchParams.get(key))fail('scope_drift');
      const cursor=value.pagination.nextCursor;
      if(cursor!==undefined&&typeof cursor!=='string')fail('invalid_cursor');
      if(value.pagination.hasMore&&(!cursor||cursor.length>4096)||!value.pagination.hasMore&&!!cursor)fail('invalid_cursor');
      if(!validatedPages.has(value)){validatedPages.add(value);diagnostic.successful_pages++;}
      return value;
    }
    function owner(value) {
      const bot=value.data.filter(row=>row?.product==='grok-bot');
      if(!bot.length)fail('owner_unavailable');
      const ids=bot.map(row=>{if(row.principal?.type!=='user'||!text(row.principal.id))fail('owner_unavailable');return row.principal.id;});
      if(!ids.every(id=>id===ids[0]))fail('owner_changed');return ids[0];
    }
      const deadline=Date.now()+settings.timeoutSeconds*1000;
      let calls=0,bytes=0;const cursors=new Set(),ids=new Set(),records=[];
      async function fetchPage(url,reserve){
        if(bytes>=8388608)fail('byte_limit');
        if(calls>=settings.maxPages-reserve)fail('page_limit');
        if(Date.now()>=deadline)fail('collection_deadline');
        calls++;diagnostic.http_status=null;
        const requestDeadline=Math.min(deadline,Date.now()+60000);
        let result;
        try{
          const response=await Reflect.apply(original,window,[url.href,{method:'GET',credentials:'same-origin',redirect:'error',signal:AbortSignal.timeout(Math.max(1,requestDeadline-Date.now()))}]);
          result=await read(response,requestDeadline,Math.min(262144,8388608-bytes));
        }catch(error){
          if(['http_auth','http_permission','http_rate_limit','http_error','http_server','response_not_json','response_schema','byte_limit'].includes(error?.message))throw error;
          if(Date.now()>=deadline)fail('collection_deadline');
          if(Date.now()>=requestDeadline||error?.name==='AbortError'||error?.name==='TimeoutError'||error?.message==='collection_deadline')fail('page_request_deadline');
          fail('http_transport');
        }
        bytes+=result.size;if(bytes>8388608)fail('byte_limit');return page(result.value,url);
      }
      // Resource timing supplies only the actual UI URL/range. GET method and
      // user/page contract are separately verified; never infer them from timing.
      function initialUrls(){return [...new Set(performance.getEntriesByType('resource').map(entry=>entry.name).filter(raw=>{try{const url=new URL(raw);return url.origin==='https://grok.com'&&url.pathname===path&&!url.searchParams.has('cursor');}catch{return false;}}))];}
      let urls=initialUrls();const discoveryDeadline=Math.min(deadline,Date.now()+10000);
      while(!urls.length&&Date.now()<discoveryDeadline){await new Promise(resolve=>setTimeout(resolve,100));urls=initialUrls();}
      if(urls.length!==1)fail('baseline_unavailable');
      const baseline=context(urls[0]);baseline.searchParams.set('pageSize','100');context(baseline.href);urls=null;
      yield {pending:true};
      browserStage='owner';
      let initial=await fetchPage(baseline,1);
      yield {pending:true};
      // Bind a Bot owner before processing the requested walk. Leading pages
      // without Bot are not an owner or terminal-zero assertion.
      const anchor=context(baseline.href),anchorCursors=new Set();
      let anchorValue=initial;
      let prefetched=[{value:initial,url:context(anchor.href)}];
      while(!anchorValue.data.some(row=>row?.product==='grok-bot')){
        if(!anchorValue.pagination.hasMore)fail('owner_unavailable');
        const cursor=anchorValue.pagination.nextCursor;if(anchorCursors.has(cursor))fail('cursor_cycle');anchorCursors.add(cursor);
        anchor.searchParams.set('cursor',cursor);anchorValue=await fetchPage(anchor,1);
        yield {pending:true};
        prefetched.push({value:anchorValue,url:context(anchor.href)});
      }
      const boundOwner=owner(anchorValue);
      const target=context(baseline.href);
      if(settings.startTime!==null){target.searchParams.set('startTime',settings.startTime);target.searchParams.set('endTime',settings.endTime);context(target.href);}
      const rangeStart=target.searchParams.get('startTime'),rangeEnd=target.searchParams.get('endTime');
      let queue=settings.startTime===null?prefetched:[];
      let chunk=queue.length?queue.shift():{value:await fetchPage(target,1),url:context(target.href)};
      yield {pending:true};
      prefetched=null;initial=null;anchorValue=null;
      browserStage='walk';
      while(true){
        if(Date.now()>=deadline)fail('collection_deadline');
        const current=chunk.value;
        page(current,chunk.url);
        for(const row of current.data){
          if(!row||typeof row!=='object'||!text(row.billingItemId)||!text(row.product))fail('invalid_page');
          if(ids.has(row.billingItemId))fail('duplicate_item');ids.add(row.billingItemId);
          if(ids.size>100000)fail('byte_limit');
          if(row.product!=='grok-bot')continue;
          if(row.principal?.type!=='user'||row.principal.id!==boundOwner)fail('owner_changed');
          if(typeof row.eventTime!=='string'||!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z$/.test(row.eventTime)||!Number.isSafeInteger(Date.parse(row.eventTime))||Date.parse(row.eventTime)<Date.parse(rangeStart)||Date.parse(row.eventTime)>=Date.parse(rangeEnd))fail('event_outside_range');
          if(!text(row.chargeCategory))fail('charge_category_unavailable');
          if(!text(row.usageType))fail('usage_type_unavailable');
          if(row.hasTokenMeters!==true)fail('token_meters_unavailable');
          if(!row.usage||typeof row.usage!=='object'||Object.keys(row.usage).length!==meters.length||meters.some(key=>!Object.hasOwn(row.usage,key)))fail('meter_schema_unavailable');
          let sum=0n;for(const key of meters){sum+=integer(row.usage[key]);if(sum>maximum)fail('meter_overflow');}if(sum!==integer(row.totalTokens))fail('total_mismatch');
          records.push({billingItemId:row.billingItemId,eventTime:row.eventTime,product:'grok-bot',chargeCategory:row.chargeCategory,usageType:row.usageType,principal:{type:'user',id:boundOwner},hasTokenMeters:true,usage:Object.fromEntries(meters.map(key=>[key,row.usage[key]])),totalTokens:row.totalTokens,model:null,cost:null});
        }
        if(!current.pagination.hasMore)break;
        const cursor=current.pagination.nextCursor;if(cursors.has(cursor))fail('cursor_cycle');cursors.add(cursor);
        if(queue.length){chunk=queue.shift();if(chunk.url.searchParams.get('cursor')!==cursor)fail('scope_drift');}
        else{target.searchParams.set('cursor',cursor);chunk={value:await fetchPage(target,1),url:context(target.href)};}
        yield {pending:true};
      }
      browserStage='recheck';
      const after=await fetchPage(anchor,0);if(owner(after)!==boundOwner)fail('owner_changed');
      yield {pending:true};
      const packet={version:1,source:'grok-dashboard-billing-v2',transport:'aside-local',scope:'user',owner:boundOwner,ownerRechecked:true,range:{startTime:rangeStart,endTime:rangeEnd},apiPaginationComplete:true,coverage:'requested_range_only',retentionKnown:false,correctionsKnown:false,collectedAt:new Date().toISOString(),records};
      const serialized=JSON.stringify(packet);
      if(new TextEncoder().encode(serialized).length>16777216)fail('byte_limit');
      globalThis.__grokHistoryPayload=serialized;
      return {ok:true,units:serialized.length};
    }catch(error){
      if(error?.message==='collection_deadline')return {ok:false,error:'collection_deadline_'+browserStage,diagnostic:details()};
      const codes=['owner_unavailable','owner_changed','baseline_unavailable','baseline_scope_invalid','invalid_range','invalid_page','invalid_cursor','cursor_cycle','duplicate_item','scope_drift','invalid_bot_record','charge_category_unavailable','usage_type_unavailable','token_meters_unavailable','meter_schema_unavailable','invalid_meter','meter_overflow','total_mismatch','event_outside_range','page_limit','byte_limit','collection_deadline','page_request_deadline','http_transport','collection_deadline_discover','collection_deadline_owner','collection_deadline_walk','collection_deadline_recheck','http_auth','http_permission','http_rate_limit','http_error','http_server','response_not_json','response_schema'];
      return {ok:false,error:codes.includes(error?.message)?error.message:browserStage==='discover'?'browser_range_runtime':browserStage==='owner'?'browser_owner_runtime':browserStage==='walk'?'browser_walk_runtime':'browser_recheck_runtime',diagnostic:details()};
    }
    })();
    return true;
  },options);
  let result;
  while(true){
    const step=await grokTab.evaluate(async()=>{
      if(!globalThis.__grokHistoryIterator)throw new Error('private_channel_invalid');
      let pending=globalThis.__grokHistoryPending;
      if(!pending){
        pending={settled:false};globalThis.__grokHistoryPending=pending;
        pending.promise=globalThis.__grokHistoryIterator.next().then(value=>{pending.result=value;pending.settled=true;},()=>{pending.result={done:true,value:{ok:false,error:'browser_runtime_unsupported'}};pending.settled=true;});
      }
      if(!pending.settled){
        let timer;
        try{await Promise.race([pending.promise,new Promise(resolve=>{timer=setTimeout(resolve,1000);})]);}finally{clearTimeout(timer);}
      }
      if(!pending.settled)return {waiting:true};
      delete globalThis.__grokHistoryPending;
      return {waiting:false,...pending.result};
    });
    yield true;
    if(step?.waiting===true)continue;
    if(!step||typeof step.done!=='boolean')throw new Error('private_channel_invalid');
    if(step.done){result=step.value;break;}
    if(step.value?.pending!==true)throw new Error('private_channel_invalid');
  }
  if(!result||!result.ok){failureDiagnostic=result?.diagnostic||null;throw new Error(result?.error||'private_channel_invalid');}
  if(!Number.isSafeInteger(result.units)||result.units<=0||result.units>16777216)throw new Error('private_channel_invalid');
  stage='encode';
  let pieces=[];
  for(let offset=0;offset<result.units;offset+=16384){
    const part=await grokTab.evaluate(index=>{if(typeof globalThis.__grokHistoryPayload!=='string')throw new Error('private_channel_invalid');return globalThis.__grokHistoryPayload.slice(index,index+16384);},offset);
    if(typeof part!=='string')throw new Error('private_channel_invalid');pieces.push(part);
    yield true;
  }
  let serialized=pieces.join('');pieces=null;
  if(serialized.length!==result.units)throw new Error('private_channel_invalid');result=null;
  await grokTab.evaluate(()=>{delete globalThis.__grokHistoryPayload;delete globalThis.__grokHistoryIterator;delete globalThis.__grokHistoryPending;});
  let body=Buffer.from(serialized,'utf8');serialized=null;
  if(body.length>16777216)throw new Error('byte_limit');
  let prefix=Buffer.alloc(4);prefix.writeUInt32BE(body.length);
  let frame=Buffer.concat([prefix,body]);
  stage='write';
  await fs.writeFile('records.pipe',frame);
  body.fill(0);frame.fill(0);body=null;frame=null;options=null;
  console.log('GROK_STATUS:complete');
} catch(error) {
  const allowed=['owner_unavailable','owner_changed','browser_dashboard_unavailable','baseline_unavailable','baseline_scope_invalid','invalid_range','invalid_page','invalid_cursor','cursor_cycle','duplicate_item','scope_drift','invalid_bot_record','charge_category_unavailable','usage_type_unavailable','token_meters_unavailable','meter_schema_unavailable','browser_runtime_unsupported','browser_range_runtime','browser_owner_runtime','browser_walk_runtime','browser_recheck_runtime','channel_encoding_unavailable','invalid_meter','meter_overflow','total_mismatch','event_outside_range','page_limit','byte_limit','collection_deadline','page_request_deadline','http_transport','collection_deadline_discover','collection_deadline_owner','collection_deadline_walk','collection_deadline_recheck','http_auth','http_permission','http_rate_limit','http_error','http_server','response_not_json','response_schema','private_channel_invalid'];
  const message=String(error?.message||'');
  const code=allowed.find(value=>message===value||message.endsWith(': '+value));
  const fallback=stage==='encode'?'channel_encoding_unavailable':stage==='write'||stage==='read_options'?'private_channel_invalid':stage==='open_dashboard'||stage==='usage_click'?'browser_dashboard_unavailable':'browser_runtime_unsupported';
  const d=failureDiagnostic;
  if(d&&typeof d==='object'&&Object.keys(d).length===4&&['http_status','phase','successful_pages','elapsed_ms'].every(key=>Object.hasOwn(d,key))&&(d.http_status===null||Number.isInteger(d.http_status)&&d.http_status>=100&&d.http_status<=599)&&['discover','owner','walk','recheck'].includes(d.phase)&&Number.isInteger(d.successful_pages)&&d.successful_pages>=0&&d.successful_pages<=512&&Number.isInteger(d.elapsed_ms)&&d.elapsed_ms>=0&&d.elapsed_ms<=3600000){
    console.log('GROK_DIAGNOSTIC:'+JSON.stringify({http_status:d.http_status,phase:d.phase,successful_pages:d.successful_pages,elapsed_ms:d.elapsed_ms}));
  }
  console.log('GROK_STATUS:error:'+(code||fallback));
} finally {
  options=null;globalThis.__grokCollectionSteps=null;
  if(globalThis.grokTab){try{await grokTab.evaluate(()=>{delete globalThis.__grokHistoryPayload;delete globalThis.__grokHistoryIterator;delete globalThis.__grokHistoryPending;});}catch{}await closeTab(grokTab);globalThis.grokTab=null;}
}

})();
