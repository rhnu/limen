const answer = await Promise.resolve(7);
export default {
  answer,
  onRequest(ctx) {
    ctx.state.answer = answer;
    ctx.state.hostApis = [typeof fetch, typeof process, typeof setTimeout];
  }
};
