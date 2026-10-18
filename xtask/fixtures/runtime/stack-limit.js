function recurse(left) {
  return left === 0 ? 0 : 1 + recurse(left - 1);
}
export default {
  onRequest(ctx) {
    if (ctx.state.recurse) ctx.state.value = recurse(100000);
    else if (ctx.state.deep) {
      let nested = null;
      for (let i = 0; i < 125; i++) nested = [nested];
      ctx.body = { nested };
    } else ctx.state.value = 1;
  },
};
