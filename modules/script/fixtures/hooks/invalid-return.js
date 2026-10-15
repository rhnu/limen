export default {
  onRequest(ctx) {
    const values = [null, {}, [], 1, 'value', true];
    if (ctx.kind === 12) return { then(resolve) { resolve(); } };
    const value = values[ctx.kind % 6];
    return ctx.kind < 6 ? value : Promise.resolve(value);
  }
};
