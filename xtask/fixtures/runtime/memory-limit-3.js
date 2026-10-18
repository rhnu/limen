export default {
  onRequest() {
    throw { message: "out of memory" };
  },
};
