---
name: develop-game-server
description: GameServerのArena・騎士団戦・WAL実装をSpecificationsに合わせて変更する際に使用する。
---

# GameServerの変更

以下のパスはリポジトリルート基準。隣接する `../Specifications` の最新仕様と実装を確認してから変更する。参照先がない場合は仕様の所在を確認し、READMEやこのskillを仕様の代用にしない。

## 参照と実装境界

Specifications/design/server/ 配下の game_server.md、guild_battle.md、guild_battle_lifecycle.md と design/system/api_transport.md・api_request_validation.md を対象機能に応じて読む。通信型を変更するときはSpecificationsのprotoと各利用側のコピー・生成設定を照合する。

- `src/application.rs` がPublicAPI/Coordinator向けRPCの入口。認証済みPeerのRoleとContextの検証を維持する。
- Arenaは `arena.rs`、master変換は `master_conversion.rs` / `master_files.rs`。GameLogicの型変更はsubmoduleと利用側を合わせて確認する。
- `rarity.rs` はmasterdata外からレアリティを供給する仮実装境界。masterから推定したり、欠損時に既定値を補わない。
- 騎士団戦は `capacity.rs` / `worker.rs` / `wal.rs` / `replay_record.rs`。現在Preloadは503で受付を閉じている。DB開戦Transaction・耐久化・戦闘runtimeが接続される前にreadyを有効化しない。
- WALのEvent HashとLocal Record Hashを区別する。署名対象、連番、前Hash、Raw Envelopeは `replay_local_record_encoding.md` を正本とする。成功応答は耐久化確認後。Battle threadでI/Oしない。
- WAL領域はInstance専用・単一Writerとし、容量と件数を明示する。復旧未確認のファイルを削除しない。Windowsビルド成功はUnixのdirectory fsync検証を意味しない。
- GameLogicの更新が必要なら、別リポジトリでcommit後にgitlinkを更新する。pushが許可された場合は参照先commitを先に公開する。

## 検証と引継ぎ

既存テストが対象機能を検証する場合は実行し、`cargo build --offline --locked`、`cargo clippy --offline --locked -- -D warnings`、`cargo fmt --check` で確認する。依存未取得なら無断で外部取得せず、必要なものを示す。新規テストの作成は適用されるAGENTS.mdとユーザー指示に従う。

READMEの実装済み機能と未接続箇所を実コードに合わせる。運用環境が必要な未実施検証をビルド結果と区別して報告する。skill自体は依存追加、DB migration実行、pushやデプロイへの許可を与えない。
