import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve} from 'node:path';
import {lookup} from 'node:dns/promises';
import {HOST,startNetworkFixture} from './network-fixture.mjs';
import {observeExit,stopRobot,traceShutdown} from './network-shutdown.mjs';
import {networkConfig} from './network-config.mjs';
const config=networkConfig();
const temporary=await mkdtemp(resolve(tmpdir(),'core-explorer-minimal-')),children=[];let fixture;
const launch=(cmd,args,env=process.env)=>{const child=spawn(cmd,args,{env,stdio:['ignore','pipe','pipe']});children.push(child);return child;};
const ready=child=>new Promise((resolve,reject)=>{let buffer='';const timer=setTimeout(()=>reject(new Error('readiness timeout')),40000);child.stdout.on('data',chunk=>{buffer+=chunk;if(buffer.length>65536){clearTimeout(timer);reject(new Error('output limit'));return;}if(buffer.includes('\n')){clearTimeout(timer);try{resolve(JSON.parse(buffer.split('\n')[0]));}catch{reject(new Error('invalid readiness'));}}});child.once('exit',()=>{clearTimeout(timer);reject(new Error('early exit'));});});
try{
  assert.ok((await lookup(HOST,{all:true})).every(x=>x.address==='127.0.0.1'));
  const relay=launch(config.relay,[]),description=await ready(relay);description.port=Number(description.address.split('/').at(-1));fixture=await startNetworkFixture(description,description.port);
  for(let i=0;i<Number(process.env.SHUTDOWN_REPEATS||1);i++){
    const robot=launch(config.python,['robot/runner.py'],{...process.env,AUKI_SHUTDOWN_TRACE:'1',AUKI_DDS_URL:fixture.base,AUKI_DMS_URL:fixture.base,AUKI_ROBOT_AUDIENCE:fixture.base+'/robots',AUKI_ROBOT_REGISTRATION:fixture.registration,AUKI_IDENTITY_FILE:resolve(temporary,'identity')});traceShutdown(robot);assert.equal((await ready(robot)).state,'ready');console.log(JSON.stringify({minimal:i+1,state:'ready'}));await stopRobot(robot,fixture);
  }
}finally{
  for(const child of children.reverse()){if(child.exitCode!==null||child.signalCode!==null)continue;const exit=observeExit(child);child.kill('SIGINT');let timer;await Promise.race([exit,new Promise(r=>{timer=setTimeout(r,2000);})]);clearTimeout(timer);if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await exit;}}
  await fixture?.close();await rm(temporary,{recursive:true,force:true});
}
