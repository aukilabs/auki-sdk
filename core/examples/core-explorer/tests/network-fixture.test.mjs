import assert from 'node:assert/strict';
import {generateKeyPairSync,sign} from 'node:crypto';
import {test} from 'node:test';
import {DOMAIN} from './fixture.mjs';
import {startNetworkFixture} from './network-fixture.mjs';
function peerId(bytes){
  let n=BigInt('0x'+bytes.toString('hex')),out='';const alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  while(n){out=alphabet[Number(n%58n)]+out;n/=58n;}
  for(const byte of bytes){if(byte)break;out='1'+out;}return out;
}
test('renewal advances leases and discovery excludes expired advertisements',async()=>{
  const keys=generateKeyPairSync('ed25519');
  const publicKey=Buffer.concat([Buffer.from([8,1,18,32]),keys.publicKey.export({type:'spki',format:'der'}).subarray(-32)]);
  const peer=peerId(Buffer.concat([Buffer.from([0,36]),publicKey]));
  const fixture=await startNetworkFixture({peer,port:12345},12346);
  try{
    let token='synthetic-service';
    const request=async(path,method='GET',body)=>{
      const response=await fetch(fixture.base+path,{method,headers:{Authorization:'Bearer '+token,'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});
      assert.ok(response.ok,'fixture request failed');return response.status===204?null:response.json();
    };
    const prefix=`/api/v1/domains/${DOMAIN}/p2p`;
    const challenge=await request(prefix+'/challenge','POST',{peer_id:peer,public_key:publicKey.toString('base64url')});
    const grant=await request(prefix+'/verify','POST',{challenge_id:challenge.challenge_id,signature:sign(null,Buffer.from(challenge.challenge,'base64url'),keys.privateKey).toString('base64url')});
    token=grant.p2p_access_token;
    const snapshot=await request('/relay-bookings','POST',{});
    const stored=fixture.bookings.get(peer);
    stored.authority_expires_at=new Date(Date.now()-1000).toISOString();
    stored.slots[0].provider_lease_expires_at=stored.authority_expires_at;
    const renewed=await request(`/relay-bookings/${snapshot.booking_id}/renew`,'POST');
    assert.equal(renewed.booking_id,snapshot.booking_id);
    assert.ok(Date.parse(renewed.authority_expires_at)>Date.now()+290000);
    assert.ok(Date.parse(renewed.slots[0].provider_lease_expires_at)>Date.now()+170000);
    assert.equal(fixture.state.renewed,1);
    await request(prefix+'/advertisements','PUT',{protocols:['/fixture/echo'],routes:[]});
    assert.equal((await request(prefix+'/advertisements')).advertisements.length,1);
    fixture.advertisements.get(peer).expires_at=new Date(Date.now()-1000).toISOString();
    assert.equal((await request(prefix+'/advertisements')).advertisements.length,0);
    await request(prefix+'/advertisements','DELETE');await request(`/relay-bookings/${snapshot.booking_id}`,'DELETE');
    assert.equal(fixture.state.withdrawn,1);assert.equal(fixture.state.released,1);
    assert.equal(fixture.advertisements.size,0);assert.equal(fixture.bookings.size,0);
  }finally{await fixture.close();}
});
