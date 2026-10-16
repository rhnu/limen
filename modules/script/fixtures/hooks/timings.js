export default {
    onRequest(ctx) {
        if (ctx.mode === "throw") throw new Error("synthetic");
        if (ctx.mode === "invalid") ctx.value = undefined;
        if (ctx.mode === "contract") return 1;
        ctx.done = true;
    },
    async onFinish(ctx) {
        await Promise.resolve();
        if (ctx.mode === "throw") throw new Error("synthetic");
        if (ctx.mode === "invalid") ctx.value = undefined;
        if (ctx.mode === "contract") return 1;
        ctx.done = true;
    },
};
