# ElwindUI documentation router

ElwindUIの永続ドキュメントは、答える質問ごとに責務を分ける。

| Need | Start here |
|---|---|
| 公開API、DSL、observable behavior | [`specs/README.md`](specs/README.md) |
| 内部architecture、ownership、data flow | [`design/README.md`](design/README.md) |
| 現在の実装・backend・verification | [`status/README.md`](status/README.md) |
| 実装時の技術ルール | [`agents/`](agents/) |
| Issue phase workflow | [`agent-workflow/`](agent-workflow/) |

通常はこのREADMEからcategory READMEを選び、そこから対象文書を1～少数だけ読む。
全spec/design/statusを最初から横断してはならない。

文書の依存方向とコード変更時の同期規則はルートの [`AGENTS.md`](../AGENTS.md) を正本とする。
人間向けのworkflow overviewは [`docs_only_human/issue-driven-development-workflow.md`](../docs_only_human/issue-driven-development-workflow.md) にある。
