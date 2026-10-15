import { REPO } from "./lib/github.mjs";

export { REPO };
export const SITE_URL = "https://hatoba.fog.moe";
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases`;
export const BUG_REPORT_URL = `${REPO_URL}/issues/new?template=bug_report.yml`;
export const SECURITY_URL = `${REPO_URL}/security/policy`;
export const CONTRIBUTING_URL = `${REPO_URL}/blob/main/CONTRIBUTING.md`;
export const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;
export const SECURITY_MODEL_URL = `${REPO_URL}/blob/main/docs/hatoba-spec.md#4-security-model-and-encryption`;
export const BUILD_FROM_SOURCE_URL = `${REPO_URL}#build-from-source`;
