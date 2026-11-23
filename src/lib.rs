pub mod application;
pub mod arena;
pub mod capacity;
pub mod config;
pub mod master_conversion;
pub mod master_files;
pub mod rarity;
pub mod rpc;
pub mod transport;
pub mod wal;
pub mod worker;
pub mod protocol {
    pub mod game {
        pub mod model {
            include!(concat!(env!("OUT_DIR"), "/game.model.rs"));
        }
        pub mod publicapi {
            include!(concat!(env!("OUT_DIR"), "/game.publicapi.rs"));
        }
        pub mod guildbattle {
            pub mod replay {
                include!(concat!(env!("OUT_DIR"), "/game.guildbattle.replay.rs"));
            }
        }
        pub mod privateapi {
            include!(concat!(env!("OUT_DIR"), "/game.privateapi.rs"));
        }
    }
}
pub mod replay_record;
