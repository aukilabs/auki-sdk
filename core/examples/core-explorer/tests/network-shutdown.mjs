import assert from 'node:assert/strict';
// Native relay cleanup:150s; protocol/node:30s each; remaining joins/HTTP:60s.
export const SHUTDOWN_BOUND_MS=270000;
export const observeExit=child=>child.exitCode!==null||child.signalCode!==null
  ? Promise.resolve({winner:'exit',code:child.exitCode,signal:child.signalCode})
  : new Promise(resolve=>child.once('exit',(code,signal)=>resolve({winner:'exit',code,signal})));
export function traceShutdown(child){
  let buffer='';child.stderr.on('data',chunk=>{buffer+=chunk;if(buffer.length>65536){buffer='';return;}let end;
    while((end=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,end);buffer=buffer.slice(end+1);try{const v=JSON.parse(line);
      if(['signal receipt','waiter gather','Echo close','task close','credential close','serve return','asyncio.run return'].includes(v.phase)&&['received','begin','end','failed','settled'].includes(v.status)&&Number.isFinite(v.elapsed))console.log(JSON.stringify({phase:v.phase,status:v.status,elapsed:v.elapsed}));
    }catch{}}
  });
}
export async function stopRobot(child,fixture,bound=SHUTDOWN_BOUND_MS){
  const exit=observeExit(child),start=performance.now(),kill=child.kill('SIGTERM');let timer;
  const result=await Promise.race([exit,new Promise(resolve=>{timer=setTimeout(()=>resolve({winner:'timeout',code:child.exitCode,signal:child.signalCode}),bound);})]);clearTimeout(timer);
  console.log(JSON.stringify({shutdown:{kill,...result,elapsed:Math.round(performance.now()-start),advertisements:fixture.advertisements.size,bookings:fixture.bookings.size,withdrawn:fixture.state.withdrawn,released:fixture.state.released,claims:fixture.state.claims}}));
  assert.equal(result.winner,'exit','robot exceeded native cleanup contract');assert.equal(result.signal,null,'signal exit is not graceful');assert.equal(result.code,0,'robot cleanup failed');
  assert.equal(fixture.advertisements.size,0);assert.equal(fixture.bookings.size,0);assert.ok(fixture.state.withdrawn>0);assert.ok(fixture.state.released>0);assert.equal(fixture.state.claims,0);
}
