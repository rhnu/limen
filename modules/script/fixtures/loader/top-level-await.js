const answer = await Promise.resolve(7);
export default {
  answer,
  onRequest(ctx) {
    ctx.answer = answer;
    ctx.hostApis = [typeof fetch, typeof process, typeof setTimeout];
  }
};
