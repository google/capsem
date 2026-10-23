import {beforeEach,expect,it,vi} from 'vitest';
import {SecurityAskDecision} from '@capsem/sdk';
const request=vi.fn();
vi.stubGlobal('fetch',request);vi.stubGlobal('WebSocket',class {close(){}});
const api=await import('../api');
const json=(value:unknown,status=200)=>new Response(JSON.stringify(value),{status});
beforeEach(async()=>{
  request.mockReset();
  request.mockResolvedValueOnce(json({ok:true,version:'1',service_socket:'/tmp/s'}))
    .mockResolvedValueOnce(json({token:'synthetic-test-token'}))
    .mockResolvedValueOnce(json({service:'running',gateway_version:'1',vm_count:0,vms:[],resource_summary:null}));
  await api.init();request.mockReset();
});
it.each([401,409,503])('never replays an approval decision after HTTP %s',async(status)=>{
  request.mockResolvedValueOnce(json({error:'unavailable'},status));
  await expect(api.resolveSecurityAsk('vm-a','ask-a','generation-a',SecurityAskDecision.APPROVE)).rejects.toMatchObject({status});
  expect(request).toHaveBeenCalledTimes(1);
  const [url,options]=request.mock.calls[0];
  expect(new URL(url).pathname).toBe('/vms/vm-a/security/asks/ask-a/resolve');
  expect(options.method).toBe('POST');
  expect(JSON.parse(options.body)).toEqual({generation:'generation-a',decision:'approve'});
});
it('reads a bounded authoritative request list',async()=>{
  const list={vm_id:'vm-a',generation:'generation-a',pending_count:0,requests:[]};
  request.mockResolvedValueOnce(json(list));expect(await api.listSecurityAsks('vm-a')).toEqual(list);
  expect(new URL(request.mock.calls[0][0]).searchParams.get('limit')).toBe('100');
});
