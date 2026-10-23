import {expect,it} from 'vitest';
import {formatCost} from '../format';
it('retains a real subcent estimate instead of displaying zero',()=>{
 expect(formatCost(0.00438)).toBe('$0.00438');
 expect(formatCost(0.000001)).toBe('$0.000001');
 expect(formatCost(0.00000001)).toBe('<$0.000001');
 expect(formatCost(0)).toBe('$0.00');
 expect(formatCost(1.23)).toBe('$1.23');
});
it('does not render invalid estimates as money',()=>{
 for(const cost of [NaN,Infinity,-1])expect(formatCost(cost)).toBe('--');
});
