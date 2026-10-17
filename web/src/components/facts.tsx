import type { ReactNode } from "react";

export function Facts({ entries }: { entries: [string, ReactNode][] }) {
    return (
        <dl className="facts">
            {entries.map(([key, value]) => (
                <div key={key}>
                    <dt>{key}</dt>
                    <dd>{value ?? "未知"}</dd>
                </div>
            ))}
        </dl>
    );
}
