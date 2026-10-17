let release;
let changes = 0;
let started = 0;
export default {
  async onRequest(ctx) {
    started++;
    ctx.state.draft = 'uncommitted';
    await new Promise(resolve => { release = resolve; });
    changes++;
    ctx.state.draft = 'late';
  },
  onFinish() { release(); },
  onError(ctx) { ctx.state.changes = changes; ctx.state.started = started; }
};
