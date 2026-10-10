
import { pathToFileURL } from 'node:url';

// returns true if the passsed file is being invoked from node,
// not imported.
export function isMain(importMetaUrl) {
    return   importMetaUrl === pathToFileURL(process.argv[1]).href;
}
// The build time stamped into packaged metadata: SOURCE_DATE_EPOCH (seconds, the reproducible-builds
// convention) when set, so two packs of one commit are byte-identical; otherwise now.
export function buildTimeIso(env = process.env, now = () => new Date()) {
    const epoch = env.SOURCE_DATE_EPOCH
    if (epoch && /^\d+$/.test(epoch)) return new Date(Number(epoch) * 1000).toISOString()
    return now().toISOString()
}
