import type {ImageInventoryResponse,ImagePackageInventory} from '@capsem/sdk';

export type InventoryView = {status:'loading'} | {status:'unavailable'} | {status:'error'} |
  {status:'ready';inventory:ImagePackageInventory};
type ReadInventory=(name:string,image:string,signal:AbortSignal)=>Promise<ImageInventoryResponse>;

/** Own only one visible Info request; late or closed reads cannot replace its state. */
export class ImageInventoryReader {
  private request:AbortController|null=null;
  constructor(private readonly read:ReadInventory,private readonly changed:(state:InventoryView)=>void) {}
  async load(name:string,image:string,architecture:string) {
    this.request?.abort();
    const request=new AbortController();this.request=request;
    this.changed({status:'loading'});
    try {
      const response=await this.read(name,image,request.signal);
      if(request.signal.aborted)return;
      if(response.image!==image || response.architecture!==architecture)throw Error('Image changed');
      if(!response.inventory) {
        if(response.artifact_reference)throw Error('Incomplete inventory');
        this.changed({status:'unavailable'});return;
      }
      if(!response.artifact_reference || response.inventory.architecture!==architecture ||
        response.inventory.schema!=='capsem.package_inventory.v1')throw Error('Invalid inventory');
      this.changed({status:'ready',inventory:response.inventory});
    } catch {
      if(!request.signal.aborted)this.changed({status:'error'});
    }
  }
  dispose(){this.request?.abort();}
}
