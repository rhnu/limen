let release;
let started = false;
export default {
  async onRequest(ctx) {
    switch (ctx.meta.operation) {
      case 'edit': ctx.event.text += ' edited'; break;
      case 'replace': ctx.event = { text: 'replacement' }; break;
      case 'drop': ctx.replaceEvents([]); break;
      case 'split': ctx.replaceEvents([{text:'a'}, {text:'b'}]); break;
      case 'twice': ctx.replaceEvents([]); ctx.replaceEvents([]); break;
      case 'invalid': ctx.replaceEvents([null]); break;
      case 'sparse': ctx.replaceEvents(new Array(1)); break;
      case 'count': ctx.replaceEvents(new Array(65).fill({})); break;
      case 'late-count': {
        const events = [{}]; ctx.replaceEvents(events);
        await Promise.resolve(); events.length = 65; break;
      }
      case 'readonly': ctx.replaceEvents = () => {}; break;
      case 'return': ctx.replaceEvents([]); return {};
      case 'reject': ctx.replaceEvents([]); throw new Error('synthetic');
      case 'pending':
        ctx.replaceEvents([{text:'uncommitted'}]);
        await new Promise(resolve => { started = true; release = resolve; });
        ctx.state.late = true; break;
      case 'missing': if (typeof ctx.replaceEvents !== 'undefined') throw new Error(); break;
    }
    ctx.state.visited = true;
  },
  onFinish() { release(); },
  onError(ctx) { ctx.state.started = started; }
};
