#[tokio::main]
async fn main() -> std::process::ExitCode {
    match game_server::config::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("GameServer startup or server failure");
            std::process::ExitCode::FAILURE
        }
    }
}
