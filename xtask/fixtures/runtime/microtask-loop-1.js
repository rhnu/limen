import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  gateway.onRequest(function () {
    function spin() {
      Promise.resolve().then(spin);
    }
    spin();
    return new Promise(() => {});
  });
});
