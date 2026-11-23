# GameServer

ReHydrangeaOdysseyのゲーム再現プロジェクトを構成するサーバーコンポーネントです。仕様の正本は[Specifications](https://github.com/ReHydrangeaOdyssey/Specifications)です。

## 実装済みの機能・基盤

- HTTP/2・TLS 1.3・mTLSによる内部RPC、呼出元Roleの検証。
- Arena編成登録、対戦相手選択、GameLogicによる戦闘計算と初期Snapshot返却。
- Protobuf master読込・型変換、外部レアリティ供給用の仮実装境界。
- 騎士団戦の容量管理、専用Worker、Ed25519署名付きWALと容量・件数制限。

## 現在の制限

騎士団戦のPreloadから開戦・戦闘・DB永続化までの接続は未完了です。Preloadは503を返し、受入可能状態にはしません。WALのDB／Replay File照合と復旧完了後の削除も未接続です。

## 開発

GameLogicを[git submodule](.gitmodules)として参照します。ビルドにはgitlinkが指すcommitの取得が必要です。

リポジトリルートで次を実行します。未取得の依存がある場合、offline実行は失敗します。

```sh
cargo build --offline --locked
cargo clippy --offline --locked -- -D warnings
cargo fmt --check
```

起動設定は [src/config.rs](src/config.rs) を参照してください。設定JSONのパスを第1引数として渡します。証明書・秘密鍵・認証情報をリポジトリへ保存しないでください。

今後の実装作業向けskill: [develop-game-server](.agents/skills/develop-game-server/SKILL.md)。実装済みの範囲はソースと最新仕様を併せて確認してください。
