import { token, frozen } from './token.mjs';
const unicode = await Promise.resolve(limen.env('LIMEN_PROBE_ENV_UNICODE'));
export default {
  providers: { p: { auth: { bearer: token } } },
  frozen,
  unicode,
  missing: limen.env('LIMEN_PROBE_ENV_MISSING') === undefined,
  empty: limen.env('LIMEN_PROBE_ENV_EMPTY'),
  async onRequest(ctx) {
    await Promise.resolve();
    if (ctx.action === 'read') {
      ctx.token = limen.env('LIMEN_PROBE_ENV_TOKEN');
      ctx.fallback = limen.env('LIMEN_PROBE_ENV_MISSING') ?? 'fallback';
      return;
    }
    if (ctx.action === 'uncaught') { limen.env('A=B'); return; }
    const rejected = (run, type) => {
      try { run(); } catch (error) {
        if (!(error instanceof type)) throw new Error('wrong error type');
        return;
      }
      throw new Error('invalid environment access succeeded');
    };
    for (const name of ['', 'A=B', 'A\0B']) {
      rejected(() => limen.env(name), TypeError);
    }
    for (const name of [null, undefined, 1, {}, { toString() { throw new Error(); } }]) {
      rejected(() => limen.env(name), TypeError);
    }
    rejected(() => limen.env(), TypeError);
    rejected(() => limen.env('x'.repeat(1025)), RangeError);
    rejected(() => limen.env('LIMEN_PROBE_ENV_LARGE'), RangeError);
    rejected(() => limen.env('LIMEN_PROBE_ENV_INVALID'), Error);
    rejected(() => { limen.env = () => 'changed'; }, TypeError);
    rejected(() => { globalThis.limen = {}; }, TypeError);
    rejected(() => { delete limen.env; }, TypeError);
    rejected(() => { delete globalThis.limen; }, TypeError);
    rejected(() => { limen.extra = true; }, TypeError);
    rejected(() => Object.setPrototypeOf(limen, {}), TypeError);
    ctx.checked = true;
  }
};
