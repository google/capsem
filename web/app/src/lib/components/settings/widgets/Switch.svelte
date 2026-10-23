<script lang="ts">
  let {id, label, checked, disabled=false, onchange}: {
    id?:string; label:string; checked:boolean; disabled?:boolean;
    onchange:(checked:boolean)=>void;
  }=$props();
</script>
<button {id} type="button" role="switch" aria-label={label} aria-checked={checked} {disabled}
  class="relative inline-flex shrink-0 h-6 w-11 border-2 border-transparent rounded-full cursor-pointer transition-colors duration-200
    {checked ? 'bg-primary' : 'bg-surface-2'} {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
  onclick={()=>{if(!disabled)onchange(!checked);}}>
  <span class="pointer-events-none inline-block size-5 rounded-full bg-white shadow-sm ring-0 transition-transform duration-200
    {checked ? 'translate-x-5' : 'translate-x-0'}"></span>
</button>
