export default { onRequest() { function spin() { Promise.resolve().then(spin); } spin(); return new Promise(() => {}); } };
