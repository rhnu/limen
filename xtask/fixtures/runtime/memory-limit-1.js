import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  gateway.onRequest(function () {
    const retained = [];
    while (true) retained.push(new Array(8192).fill(1));
  });
});
