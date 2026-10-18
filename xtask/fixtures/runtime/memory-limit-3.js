import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  gateway.onRequest(function () {
    throw { message: "out of memory" };
  });
});
