function edit(ctx) {
  switch (ctx.meta.operation) {
    case 'edit':
      ctx.status = 503;
      ctx.body = JSON.stringify({ message: 'translated', original: ctx.body });
      ctx.contentType = 'application/json';
      ctx.headers.set('retry-after', '5');
      ctx.state.nested.value = 2;
      break;
    case 'null': ctx.body = null; ctx.contentType = null; break;
    case 'meta': ctx.meta.phase = 'onFinish'; break;
    case 'meta-ref': ctx.meta = {}; break;
    case 'error': ctx.error.code = 'changed'; break;
    case 'error-ref': ctx.error = {}; break;
    case 'state-ref': ctx.state = {}; break;
    case 'headers-ref': ctx.headers = []; break;
    case 'headers-method': ctx.headers.set = () => {}; break;
    case 'transport-header': ctx.headers.set(ctx.meta.header, 'synthetic'); break;
    case 'reject': ctx.reject(400); break;
    case 'return': return {};
    case 'async-return': return Promise.resolve({});
    case 'async-edit':
      return Promise.resolve().then(() => { ctx.body = 'asynchronous translation'; });
  }
}
export default {
  onErrorResponse: edit,
  providers: { p: { pipeline: [{
    name: 'provider-error',
    onErrorResponse(ctx) { ctx.state.order.push('provider'); }
  }] } },
  models: { m: { pipeline: [{
    name: 'model-error',
    async onErrorResponse(ctx) {
      await Promise.resolve();
      ctx.state.order.push('model');
    }
  }] } }
};
