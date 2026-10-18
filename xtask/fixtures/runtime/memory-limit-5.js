export default {
  onRequest() {
    return Promise.reject({ message: "out of memory" });
  },
};
