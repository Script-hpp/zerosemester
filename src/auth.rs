use axum::{extract::Query, response::Html, routing::get, Router};
use serde::Deserialize;
use std::sync::Arc;
use tokio::{net::TcpListener, sync::{oneshot, Mutex}};
use oauth2::{
basic::BasicClient, AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl,CsrfToken, TokenResponse
};
use dotenvy::dotenv;
use std::env;

#[derive(Deserialize)]
struct AuthCallback {
    code: String,
}

pub async fn authenticate_with_notion() {

    dotenv().ok(); // Load environment variables from .env file
    
    let client_id = ClientId::new(env::var("NOTION_CLIENT_ID").unwrap_or_else(|_| "YOUR_CLIENT_ID".into()));
    let client_secret = ClientSecret::new(env::var("NOTION_CLIENT_SECRET").unwrap_or_else(|_| "YOUR_CLIENT_SECRET".into()));
    let auth_url = AuthUrl::new("https://api.notion.com/v1/oauth/authorize".to_string())
        .expect("Invalid authorization endpoint URL"); 
    let token_url = TokenUrl::new("https://api.notion.com/v1/oauth/token".to_string())
        .expect("Invalid token endpoint URL");
    let redirect_url = RedirectUrl::new("http://localhost:8080/callback".to_string())
        .expect("Invalid redirect URL");

    let client = BasicClient::new(
        client_id,
        Some(client_secret),
        auth_url,
        Some(token_url),
    )
    .set_redirect_uri(redirect_url);

    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .url();

    let (tx, rx) = oneshot::channel::<String>();

    let shared_tx = Arc::new(Mutex::new(Some(tx)));

    let app = Router::new().route(
        "/callback",
        get({
            let shared_tx = Arc::clone(&shared_tx);
            move |Query(query): Query<AuthCallback>| async move {
                if let Some(tx) = shared_tx.lock().await.take() {
                    let _ = tx.send(query.code.clone());
                }
                Html("<h1>Authentication successful! You can close this window.</h1>")
            }
        }),
    );

    tokio::spawn(async move {
        match TcpListener::bind("127.0.0.1:8080").await {
            Ok(listener) => {
                if let Err(e) = axum::serve(listener, app).await {
                    eprintln!("Server error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Could not bind to port 8080: {}. Is it already in use?", e);
            }
        }
    });


    
    if webbrowser::open(auth_url.as_str()).is_err() {
        println!("Failed to open browser for authentication. Please complete the process manually.");
    }

    if let Ok(code) = rx.await {
        println!("Received authorization code: {}", code);

        let token_result = client
            .exchange_code(oauth2::AuthorizationCode::new(code))
            .request_async(oauth2::reqwest::async_http_client)
            .await;
        
        match token_result {
            Ok(token) => {
                let access_token = token.access_token().secret();
                std::fs::write("notion_token.txt", access_token).expect("Failed to write token to file");
            }
            Err(err) => {
                std::fs::write("error.txt", format!("Fehler beim Login: {:?}", err)).unwrap();      
            }
        }
    } else 
    {
        println!("Failed to receive authorization code.");
    }
    
}
