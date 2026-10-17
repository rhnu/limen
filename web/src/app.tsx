import { lazy, Suspense, useEffect, useState } from "react";
import {
    HashRouter,
    Link,
    NavLink,
    Outlet,
    Route,
    Routes,
    useLocation,
    useNavigate,
} from "react-router";
import { Button } from "@radix-ui/themes";
import { LogOut } from "lucide-react";
import { validateKey } from "./api/client";
import { isCurrentSession, replaceSession, useSession } from "./lib/session";
import { Failure, Loading } from "./components/common";
import { Login } from "./components/login";
import { Home } from "./pages/home";
const Usage = lazy(() => import("./pages/usage").then((module) => ({ default: module.Usage })));
const Requests = lazy(() =>
    import("./pages/requests").then((module) => ({ default: module.Requests })),
);
const RequestDetail = lazy(() =>
    import("./pages/requests").then((module) => ({ default: module.RequestDetail })),
);
const Logs = lazy(() => import("./pages/logs").then((module) => ({ default: module.Logs })));
const Status = lazy(() => import("./pages/status").then((module) => ({ default: module.Status })));
const Config = lazy(() => import("./pages/config").then((module) => ({ default: module.Config })));

let restorePromise: Promise<void> | null = null;
let restoreFailure: unknown = null;
export function restoreSession() {
    if (restorePromise) return restorePromise;
    const { key, epoch, phase } = useSession.getState();
    if (!key || phase !== "verifying") return Promise.resolve();
    restoreFailure = null;
    restorePromise = validateKey(key, epoch)
        .then(() => {
            if (isCurrentSession(epoch)) useSession.setState({ phase: "authenticated" });
        })
        .catch((error) => {
            if (!isCurrentSession(epoch)) return;
            if (error?.status === 401) {
                replaceSession(null, "public");
                useSession.setState({ authError: "管理密钥已失效，请重新登录。" });
            } else {
                restoreFailure = error;
                useSession.setState({ phase: "blocked" });
            }
        })
        .finally(() => {
            restorePromise = null;
        });
    return restorePromise;
}
function Gate() {
    const phase = useSession((state) => state.phase);
    const location = useLocation();
    const navigate = useNavigate();
    const [open, setOpen] = useState(true);
    useEffect(() => {
        if (phase === "public") setOpen(true);
    }, [phase]);
    if (phase === "authenticated") return <Layout />;
    return (
        <main className="gate">
            <Link className="wordmark" to="/">
                LIMEN
            </Link>
            {phase === "verifying" ? (
                <Loading />
            ) : phase === "blocked" ? (
                <>
                    <h1>管理会话</h1>
                    <Failure
                        error={restoreFailure}
                        reset={() => {
                            useSession.setState({ phase: "verifying" });
                            void restoreSession();
                        }}
                    />
                    <Button variant="outline" mt="4" onClick={() => replaceSession(null, "public")}>
                        更换密钥
                    </Button>
                </>
            ) : (
                <>
                    <h1>管理登录</h1>
                    <Button variant="outline" onClick={() => setOpen(true)}>
                        登录
                    </Button>
                    <Login
                        gate
                        open={open}
                        destination={location.pathname + location.search}
                        onOpenChange={(value) => {
                            setOpen(value);
                            if (!value) navigate("/", { replace: true });
                        }}
                    />
                </>
            )}
        </main>
    );
}
function Layout() {
    const navigate = useNavigate();
    return (
        <>
            <header className="admin-header">
                <div className="admin-header-inner">
                    <Link className="wordmark" to="/">
                        LIMEN
                    </Link>
                    <nav aria-label="管理导航">
                        {[
                            ["/usage", "用量"],
                            ["/requests", "请求记录"],
                            ["/logs", "运行日志"],
                            ["/status", "系统状态"],
                            ["/config", "配置"],
                        ].map(([to, name]) => (
                            <NavLink
                                key={to}
                                to={to}
                                className={({ isActive }) => (isActive ? "active" : "")}
                            >
                                {name}
                            </NavLink>
                        ))}
                    </nav>
                    <Button
                        className="logout"
                        variant="ghost"
                        onClick={() => {
                            replaceSession(null, "public");
                            navigate("/");
                        }}
                    >
                        <LogOut size={15} aria-hidden="true" />
                        退出
                    </Button>
                </div>
            </header>
            <main className="admin-main">
                <Suspense fallback={<Loading />}>
                    <Outlet />
                </Suspense>
            </main>
        </>
    );
}
function NotFound() {
    return (
        <main className="gate">
            <h1>页面不存在</h1>
            <Link to="/">返回首页</Link>
        </main>
    );
}
export function App() {
    const storageError = useSession((state) => state.storageError);
    useEffect(() => {
        void restoreSession();
    }, []);
    return (
        <HashRouter>
            {storageError && (
                <div className="global-notice" role="alert">
                    {storageError}
                </div>
            )}
            <Routes>
                <Route path="/" element={<Home />} />
                <Route element={<Gate />}>
                    <Route path="/usage" element={<Usage />} />
                    <Route path="/requests" element={<Requests />} />
                    <Route path="/requests/:id" element={<RequestDetail />} />
                    <Route path="/logs" element={<Logs />} />
                    <Route path="/status" element={<Status />} />
                    <Route path="/config" element={<Config />} />
                </Route>
                <Route path="*" element={<NotFound />} />
            </Routes>
        </HashRouter>
    );
}
