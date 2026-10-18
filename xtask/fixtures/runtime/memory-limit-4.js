import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  gateway.onRequest(function () {
    return Promise.reject(null);
  });
});
