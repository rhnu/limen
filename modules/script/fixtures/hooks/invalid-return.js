export default {
  onRequest(ctx) {
    const values = [null, {}, [], 1, 'value', true];
    if (ctx.state.kind === 12) return { then(resolve) { resolve(); } };
    const value = values[ctx.state.kind % 6];
    return ctx.state.kind < 6 ? value : Promise.resolve(value);
  }
};
