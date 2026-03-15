pub mod alsa_volume;
pub mod audio_sink;
pub mod audio_source;
pub mod consumer_server;
pub mod producer_server;
pub mod protocol;
pub mod mpv_controller;
pub mod spotify_controller;
pub mod types;

// Wakeword detection modules
pub mod wakeword_error;
pub mod wakeword_model;
pub mod wakeword_models;
pub mod wakeword_utils;
pub mod wakeword_vad;

// LED ring control
pub mod led_engine;
pub mod led_ring;
