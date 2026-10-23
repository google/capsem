import {beforeEach,expect,it,vi} from 'vitest';
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
const generation='18f59e33-8af6-4992-81d9-71e10873af9a';
const body={owner_generation:generation,request_id:1};
it.each([401,409,503])('never replays browser confirmation after HTTP %s',async(status)=>{
  request.mockResolvedValueOnce(json({error:'unavailable'},status));
  await expect(api.acknowledgeBrowserRequest('vm-a',body)).rejects.toMatchObject({status});
  expect(request).toHaveBeenCalledTimes(1);
  const [url,options]=request.mock.calls[0];
  expect(new URL(url).pathname).toBe('/vms/vm-a/browser/acknowledge');
  expect(JSON.parse(options.body)).toEqual(body);
  expect(options.body).not.toContain('url');
});
it('accepts only matching owner and request receipts',async()=>{
  request.mockResolvedValueOnce(json({vm_id:'vm-a',...body}));
  await expect(api.acknowledgeBrowserRequest('vm-a',body)).resolves.toBeUndefined();
  request.mockResolvedValueOnce(json({vm_id:'vm-a',...body,owner_generation:'other'}));
  await expect(api.acknowledgeBrowserRequest('vm-a',body)).rejects.toMatchObject({status:502});
});
