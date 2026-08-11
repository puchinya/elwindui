# ElwindUI Issue-driven development workflow

Updated: 2026-08-11

この文書は人間向けの全体像である。AI Agentが実際に従うrepository-wide ruleは [`AGENTS.md`](../AGENTS.md)、phaseごとの正確な手順は [`docs/agent-workflow/`](../docs/agent-workflow/) を正本とする。この文書だけに必須ruleを置かない。

## 1. Actors and entry points

- Codexを含むAI Agentは [`AGENTS.md`](../AGENTS.md) をentry pointとする。
- Claude Codeは [`CLAUDE.md`](../CLAUDE.md) から入り、共通ruleについて必ず `AGENTS.md` へ従う。
- どちらも文書検索は [`docs/README.md`](../docs/README.md) から開始し、category READMEを経て必要な1～少数の文書を選ぶ。

CodexとClaude Codeで異なるのはentry pointだけで、Issue workflow、document authority、GitHub CLI、product contractは共通である。

## 2. GitHub and Git tools

このrepositoryでは次に統一する。

| Operation | Tool |
|---|---|
| Issue、label、milestone、comment、Pull Request、review、Actions | `gh` |
| branch、stage、commit、push、local diff/history | `git` |

別のGitHub integrationは、ユーザーが明示した場合または`gh`で必要操作を実行できない場合だけ使う。

## 3. Issue phases

Repositoryを変更する作業は一つのGitHub Issueに紐付ける。説明・調査だけならIssueを自動作成しない。

| State | Purpose | Agent instructions |
|---|---|---|
| `phase:requirements` | 目的、scope、non-goal、acceptanceを確定 | [`requirements.md`](../docs/agent-workflow/requirements.md) |
| `phase:design` | 実装判断とtest strategyを承認可能にする | [`design.md`](../docs/agent-workflow/design.md) |
| `phase:ready` | requirements/design承認済み | [`implementation.md`](../docs/agent-workflow/implementation.md) |
| `phase:implementation` | branch上で実装・verification中 | [`implementation.md`](../docs/agent-workflow/implementation.md) |
| `phase:review` | PR review・CI対応中 | [`review.md`](../docs/agent-workflow/review.md) |

`needs-user-decision` はproduct/architecture decision待ち、`blocked` は外部または技術的な進行不能を表す。単に作業量が多いことはblockedではない。

## 4. Requirements phase

1. 既存Issue/PRと関連specを検索する。
2. 変更作業にIssueがなければ、実装前に作成する。
3. Rust workspace versionと同名のmilestoneへIssueを割り当てる。
4. background、objective、requirements、non-goals、constraints、acceptance、unresolved questionsを分離する。
5. 変更をpublic contract、internal architecture、implementation-only、bug fix、verification-onlyに分類する。
6. user decisionを勝手に決めず、要件がtestableになってから`phase:design`へ進む。

詳細とcompletion criteriaは [`requirements.md`](../docs/agent-workflow/requirements.md) を参照する。

## 5. Design phase

Issue固有designはIssue本文またはcommentへ記録する。次の作業を超えて再利用されるpublic contractやdurable architectureだけをrepository docsへ昇格させる。

Designは必要な項目だけを扱う。

- public behavior;
- responsibilities、ownership、lifetime;
- data/event flow;
- backend boundary;
- thread/async/error behavior;
- compatibility、performance、cache;
- tests、alternatives。

承認後にIssue本文をrequirements/design/acceptanceの正本へ更新し、`phase:ready`へ進む。詳細は [`design.md`](../docs/agent-workflow/design.md) を参照する。

## 6. Document authority and synchronization

永続情報の責務は次のとおりである。

| Location | Meaning |
|---|---|
| [`docs/specs/`](../docs/specs/) | normative public contract |
| [`docs/design/`](../docs/design/) | durable internal architecture |
| source code | current implementation |
| tests | executable evidence |
| [`docs/status/`](../docs/status/) | current implementation/verification summary |
| [`docs/agents/`](../docs/agents/) | technical Agent rules |

依存方向は次であり、逆向きにdesired behaviorを決めない。

```text
specs -> design -> code -> status
```

| Change | Synchronization order |
|---|---|
| Public behavior / DSL contract | spec -> design when needed -> code -> status |
| Internal architecture | design -> code -> status when needed |
| Approved implementation gap | code -> status |
| Bug fix against existing contract | code -> status when needed |
| Verification only | tests/evidence -> status |

現在のbugにspecを合わせない。設計が変わらない変更でdesignを無意味に書き換えない。実装中にcontract/design不足を発見したら、Issueをrequirements/designへ戻して承認を得る。

この表のAgent向け正本は [`AGENTS.md`](../AGENTS.md) である。

## 7. Implementation phase

実装前にIssue本文と新しいcommentを確認し、remote default branchから専用branchを作る。source変更は通常 `feature/<issue>-<slug>`、docs/workflow-onlyは `docs/` / `agent/` またはrepositoryで承認されたprefixを使う。

Implementationでは:

- approved scopeだけを変更する;
- unrelated formatting/refactorを混ぜない;
- document synchronization orderを守る;
- testとverificationを実行する;
- 未実行platformやresidual riskを正直に記録する;
- staging前に全diffをself-reviewする。

詳細は [`implementation.md`](../docs/agent-workflow/implementation.md) を参照する。

## 8. Pull Request and review

PR本文には次を含める。

- purposeとimpact;
- main changesと重要なdecision;
- verification command/result;
- untested environment;
- compatibility/residual risk;
- reviewer guidance;
- `Closes #<issue-number>`。

PR作成、review/comment取得、check確認、label更新は`gh`を使う。reviewでrequirements/design変更が必要になった場合はIssueを`phase:design`へ戻し、承認後に再実装する。

merge条件とreview処理は [`review.md`](../docs/agent-workflow/review.md) を参照する。

## 9. Checkpoints and resuming

Pause/resume時は [`checkpoint.md`](../docs/agent-workflow/checkpoint.md) とrepository helper scriptを使う。checkpointにはdecision、current branch/commit、dirty changes、verification、残作業、未完了のspec/design/code/status同期を含める。

Checkpointは新しいproduct decisionの正本ではない。再開後はIssueと現在のdiffを再確認する。

## 10. Evidence

| Evidence | Storage |
|---|---|
| local investigation logs/screenshots | `.agent-state/issues/<issue>/` |
| small durable review evidence | `docs/issues/<issue>-<slug>/evidence/` with README |
| large logs/videos/dumps/image sets | CI artifact |
| concise result and links | Issue or PR |

`.agent-state/` はgitignore対象である。secret、token、private data、不要なuser-specific pathを保存しない。raw measurementやdebugging historyを `docs/status` へ常設しない。

正確な保存ruleは [`evidence.md`](../docs/agent-workflow/evidence.md) を参照する。

## 11. Document maintenance checklist

コードまたは文書の変更後に確認する。

- public behaviorが変わったのにspecが古くないか;
- architectureが変わったのにdesignが古くないか;
- implementation/verification stateが変わったのにstatusが古くないか;
- Agent command/invariantが古くないか;
- `AGENTS.md`と`CLAUDE.md`が矛盾していないか;
- category READMEから対象文書へ到達できるか;
- removed path、broken link、古いIssue説明が残っていないか;
- human overviewにしか存在しない必須ruleがないか。
