import { fail } from './private-module.mjs';
let touched = 0;
const trap = () => { touched++; throw new Error('synthetic-secret'); };
export default {
  onRequest(ctx) {
    if (ctx.state.mode === 'import') fail();
    if (ctx.state.mode === 'async') return Promise.reject(new ReferenceError('synthetic-secret'));
    if (ctx.state.mode === 'string') throw 'synthetic-secret';
    if (ctx.state.mode === 'proxy') throw new Proxy({}, { get: trap, getPrototypeOf: trap, getOwnPropertyDescriptor: trap });
    if (ctx.state.mode === 'revoked') {
      const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); throw proxy;
    }
    if (ctx.state.mode === 'subclass') { class Custom extends RangeError {} throw new Custom('synthetic-secret'); }
    if (ctx.state.mode === 'custom-stack' || ctx.state.mode === 'unicode-stack' || ctx.state.mode === 'huge-stack') {
      Error.prepareStackTrace = () => ctx.state.mode === 'custom-stack' ? { get value() { trap(); } }
        : ctx.state.mode === 'unicode-stack' ? '\uD800' : 'x'.repeat(8192);
      const error = new Error('synthetic-secret');
      Error.prepareStackTrace = undefined;
      throw error;
    }
    const error = new Error('synthetic-secret');
    if (ctx.state.mode === 'prototype-proxy') Object.setPrototypeOf(error, new Proxy({}, { getPrototypeOf: trap, get: trap }));
    if (ctx.state.mode === 'getters') {
      for (const key of ['message', 'name', 'stack', 'constructor']) Object.defineProperty(error, key, { get: trap });
    }
    if (ctx.state.mode === 'intrinsics') {
      Object.getPrototypeOf = trap;
      Object.getOwnPropertyDescriptor = trap;
      Error.isError = trap;
    }
    throw error;
  },
  onFinish(ctx) { ctx.state.touched = touched; ctx.state.ok = true; }
};
