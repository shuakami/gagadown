#!/usr/bin/env bash
# 不用 git 命令行，直接用 GitHub API 把当前目录推到 main。
# 用法：GH_TOKEN=xxx tools/gh-push.sh "提交说明"
set -euo pipefail
cd "$(dirname "$0")/.."
: "${GH_TOKEN:?需要 GH_TOKEN}"
OWNER=${OWNER:-shuakami}
REPO=${REPO:-gagadown}
BRANCH=${BRANCH:-main}
MSG=${1:-"更新"}
NAME="Shuakami"
EMAIL="shuakami@Sdjz.wiki"
DESC="快、稳、不折腾的桌面下载器：多连接动态分段、多线路竞速、可靠续传，浏览器下载一键接管。支持 Windows 和 Linux。"
TOPICS='["download-manager","downloader","rust","egui","wgpu","multi-connection","segmented-download","resume-download","http-downloader","browser-extension","chrome-extension","idm-alternative","windows","linux","desktop-app","cleartype"]'
COLLAB=${COLLAB:-xiaoyueyoQWQ}
API="https://api.github.com"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# api METHOD PATH [JSON文件] → 响应写到 $TMP/out，返回 HTTP 状态码
api() {
  local args=(-sS -o "$TMP/out" -w '%{http_code}' -X "$1"
    -H "Authorization: Bearer $GH_TOKEN"
    -H "Accept: application/vnd.github+json"
    -H "X-GitHub-Api-Version: 2022-11-28")
  [ $# -ge 3 ] && args+=(-H "Content-Type: application/json" --data-binary "@$3")
  curl "${args[@]}" "$API$2"
}
must() { local c; c=$(api "$@"); case $c in 2*) ;; *) echo "$1 $2 失败：HTTP $c" >&2; cat "$TMP/out" >&2; exit 1;; esac; }
field() { python3 -c "import json,sys;print(json.load(open('$TMP/out'))$1)"; }
json() { python3 -c "import json,sys;print(json.dumps($1))" > "$TMP/req"; }

# 1. 检查有没有泄露的凭证
if grep -rIlE "ghp_[A-Za-z0-9]{30}|github_pat_[A-Za-z0-9_]{20}|BDUSS=|STOKEN=" \
     --exclude-dir=target --exclude-dir=dist --exclude-dir=.git --exclude='*.fdpart' --exclude=gh-push.sh . ; then
  echo "上面的文件里像是有凭证，已中止" >&2; exit 1
fi

# 2. 仓库不存在就建
FRESH=0
code=$(api GET "/repos/$OWNER/$REPO")
if [ "$code" = 404 ]; then
  json "{'name':'$REPO','description':'''$DESC''','private':False,'has_wiki':False,'has_projects':False,'auto_init':True}"
  must POST /user/repos "$TMP/req"
  FRESH=1
  echo "已创建仓库 $OWNER/$REPO"
  sleep 3
fi

# 3. 上传文件
python3 - "$TMP" <<'PY' > "$TMP/files"
import os, sys
skip_dirs = {'.git', 'target', 'dist', 'node_modules', '__pycache__'}
for root, dirs, files in os.walk('.'):
    dirs[:] = sorted(d for d in dirs if d not in skip_dirs)
    for f in sorted(files):
        if f.endswith(('.fdpart', '.pyc')): continue
        p = os.path.join(root, f)[2:]
        mode = '100755' if os.access(p, os.X_OK) else '100644'
        print(mode + '\t' + p)
PY
echo "共 $(wc -l < "$TMP/files") 个文件"
: > "$TMP/tree"
while IFS=$'\t' read -r mode path; do
  python3 -c "import base64,json,sys;print(json.dumps({'content':base64.b64encode(open(sys.argv[1],'rb').read()).decode(),'encoding':'base64'}))" "$path" > "$TMP/req"
  must POST "/repos/$OWNER/$REPO/git/blobs" "$TMP/req"
  printf '%s\t%s\t%s\n' "$mode" "$(field "['sha']")" "$path" >> "$TMP/tree"
done < "$TMP/files"

python3 -c "
import json,sys
t=[dict(zip(('mode','sha','path'),l.rstrip('\n').split('\t')),type='blob') for l in open('$TMP/tree')]
print(json.dumps({'tree':t}))" > "$TMP/req"
must POST "/repos/$OWNER/$REPO/git/trees" "$TMP/req"
TREE=$(field "['sha']")

# 4. 提交
PARENTS="[]"
if [ $FRESH = 0 ] && [ "$(api GET "/repos/$OWNER/$REPO/git/ref/heads/$BRANCH")" = 200 ]; then
  HEAD=$(field "['object']['sha']")
  PARENTS="['$HEAD']"
  must GET "/repos/$OWNER/$REPO/git/commits/$HEAD"
  [ "$(field "['tree']['sha']")" = "$TREE" ] && { echo "没有变化"; NOCHANGE=1; }
fi
if [ -z "${NOCHANGE:-}" ]; then
  NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  python3 -c "import json,sys;print(json.dumps({'message':sys.argv[1],'tree':'$TREE','parents':$PARENTS,'author':{'name':'$NAME','email':'$EMAIL','date':'$NOW'},'committer':{'name':'$NAME','email':'$EMAIL','date':'$NOW'}}))" "$MSG" > "$TMP/req"
  must POST "/repos/$OWNER/$REPO/git/commits" "$TMP/req"
  COMMIT=$(field "['sha']")
  json "{'sha':'$COMMIT','force':$([ $FRESH = 1 ] && echo True || echo False)}"
  if [ "$(api GET "/repos/$OWNER/$REPO/git/ref/heads/$BRANCH")" = 200 ]; then
    must PATCH "/repos/$OWNER/$REPO/git/refs/heads/$BRANCH" "$TMP/req"
  else
    json "{'ref':'refs/heads/$BRANCH','sha':'$COMMIT'}"
    must POST "/repos/$OWNER/$REPO/git/refs" "$TMP/req"
  fi
  echo "已推送 $COMMIT"
fi

# 5. 仓库信息、Topics、协作者
json "{'description':'''$DESC''','homepage':'https://github.com/$OWNER/$REPO/releases','has_wiki':False,'has_projects':False,'default_branch':'$BRANCH'}"
must PATCH "/repos/$OWNER/$REPO" "$TMP/req"
json "{'names':$TOPICS}"
must PUT "/repos/$OWNER/$REPO/topics" "$TMP/req"
if [ -n "$COLLAB" ]; then
  json "{'permission':'push'}"
  code=$(api PUT "/repos/$OWNER/$REPO/collaborators/$COLLAB" "$TMP/req")
  case $code in 201) echo "已邀请 $COLLAB";; 204) echo "$COLLAB 已经是协作者";; *) echo "邀请 $COLLAB 失败：HTTP $code" >&2; cat "$TMP/out" >&2;; esac
fi
echo "https://github.com/$OWNER/$REPO"
