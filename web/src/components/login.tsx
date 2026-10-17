import { useState, type FormEvent } from "react";
import { Dialog, Button } from "@radix-ui/themes";
import { useNavigate } from "react-router";
import { validateKey } from "../api/client";
import { replaceSession, useSession } from "../lib/session";
import { Failure, Field } from "./common";

export function Login({
    open,
    onOpenChange,
    destination = "/usage",
    gate = false,
}: {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    destination?: string;
    gate?: boolean;
}) {
    const [key, setKey] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<unknown>(null);
    const storageError = useSession((state) => state.storageError);
    const authError = useSession((state) => state.authError);
    const navigate = useNavigate();
    async function submit(event: FormEvent) {
        event.preventDefault();
        if (!key || busy) return;
        setBusy(true);
        setError(null);
        const epoch = replaceSession(null, "public");
        try {
            await validateKey(key, epoch);
            if (useSession.getState().epoch !== epoch) return;
            replaceSession(key, "authenticated");
            setKey("");
            onOpenChange(false);
            navigate(destination, { replace: gate });
        } catch (failure) {
            if (useSession.getState().epoch === epoch) {
                useSession.setState({ phase: "public" });
                setError(failure);
            }
        } finally {
            setBusy(false);
        }
    }
    return (
        <Dialog.Root
            open={open}
            onOpenChange={(value) => {
                if (!busy) {
                    setError(null);
                    if (!value) setKey("");
                    onOpenChange(value);
                }
            }}
        >
            <Dialog.Content maxWidth="420px">
                <Dialog.Title>管理登录</Dialog.Title>
                <Dialog.Description className="sr-only">
                    输入独立的 managerKey 访问只读管理页面。
                </Dialog.Description>
                <form onSubmit={submit} className="login-form">
                    <Field
                        label="managerKey"
                        help="管理密钥保存在此浏览器的本站存储中。退出会清除密钥；请仅在可信设备上登录。"
                    >
                        <input
                            autoComplete="off"
                            type="password"
                            value={key}
                            onChange={(event) => setKey(event.target.value)}
                            autoFocus
                            required
                            spellCheck={false}
                        />
                    </Field>
                    {error !== null && <Failure error={error} />}
                    {authError && (
                        <p role="alert" className="inline-error">
                            {authError}
                        </p>
                    )}
                    {storageError && (
                        <p role="alert" className="inline-error">
                            {storageError}
                        </p>
                    )}
                    <div className="dialog-actions">
                        <Dialog.Close>
                            <Button type="button" variant="outline" disabled={busy}>
                                取消
                            </Button>
                        </Dialog.Close>
                        <Button type="submit" disabled={busy || !key}>
                            {busy ? "正在验证…" : "登录"}
                        </Button>
                    </div>
                </form>
            </Dialog.Content>
        </Dialog.Root>
    );
}
