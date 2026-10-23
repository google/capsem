import {SecurityAskStatus, type SecurityAskDecision, type SecurityAskListResponse} from '@capsem/sdk';

export function approvalKey(list: SecurityAskListResponse, askId: string): string {
  return JSON.stringify([list.vm_id, list.generation, askId]);
}

export interface ApprovalState {
  response: SecurityAskListResponse | null;
  loading: boolean;
  resolving: string | null;
  error: string | null;
}

/** Owns a panel's read lifecycle; a decision always uses its observed generation. */
export class ApprovalQueue {
  state: ApprovalState = {response:null,loading:false,resolving:null,error:null};
  private revision=0;
  private vmId:string|null=null;
  constructor(
    private read:(vmId:string)=>Promise<SecurityAskListResponse>,
    private resolve:(vmId:string,askId:string,generation:string,decision:SecurityAskDecision)=>Promise<unknown>,
    private changed:(state:ApprovalState)=>void,
    private now:()=>number=Date.now,
  ) {}
  private publish(patch:Partial<ApprovalState>) {this.state={...this.state,...patch};this.changed(this.state);}
  async load(vmId:string):Promise<void> {
    const revision=++this.revision;
    const switched=this.vmId!==vmId;
    this.vmId=vmId;
    this.publish({loading:true,error:null,...(switched?{response:null,resolving:null}: {})});
    try {
      const response=await this.read(vmId);
      if(revision!==this.revision)return;
      if(response.vm_id!==vmId)throw Error('mismatched approval owner');
      this.publish({response});
    } catch {
      if(revision===this.revision)this.publish({response:null,error:'Approval requests unavailable. Retry to check the current state.'});
    } finally {
      if(revision===this.revision)this.publish({loading:false});
    }
  }
  async decide(askId:string,decision:SecurityAskDecision):Promise<void> {
    const list=this.state.response;
    const request=list?.requests.find(request=>request.ask_id===askId);
    if(!list || !request || this.state.resolving || this.state.loading
      || request.status!==SecurityAskStatus.PENDING || request.expires_at_unix_ms<=this.now())return;
    this.publish({resolving:askId,error:null});
    let error:string|null=null;
    try {await this.resolve(list.vm_id,askId,list.generation,decision);}
    catch(failure) {
      error=typeof failure==='object' && failure!==null && 'status' in failure && failure.status===409
        ? 'Request changed. Review its current state before deciding.'
        : 'Decision could not be confirmed. Check its current state before trying again.';
    }
    if(this.vmId!==list.vm_id)return;
    await this.load(list.vm_id);
    if(this.vmId===list.vm_id)this.publish({resolving:null,error:error??this.state.error});
  }
  dispose() {++this.revision;this.vmId=null;}
}

/** Client-local notice identity only; never persists request summaries or decisions. */
export class ApprovalNotices {
  private seen=new Set<string>();
  observe(list:SecurityAskListResponse,now=Date.now()):string[] {
    const fresh:string[]=[];
    for(const request of list.requests) {
      if(request.status!==SecurityAskStatus.PENDING || request.expires_at_unix_ms<=now)continue;
      const key=approvalKey(list,request.ask_id);
      if(this.seen.has(key))continue;
      this.seen.add(key);fresh.push(request.ask_id);
      if(this.seen.size>4096)this.seen.delete(this.seen.values().next().value!);
    }
    return fresh;
  }
}
