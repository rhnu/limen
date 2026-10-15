let release;
let changes = 0;
let started = 0;
export default {
  async onRequest(ctx) {
    started++;
    ctx.draft = 'uncommitted';
    await new Promise(resolve => { release = resolve; });
    changes++;
    ctx.draft = 'late';
  },
  onFinish() { release(); },
  onError(ctx) { ctx.changes = changes; ctx.started = started; }
};
