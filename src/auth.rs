use oauth2::{
basic::BasicClient, AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenUrl,CsrfToken, TokenResponse
};


pub fn build_oauth_client() -> BasicClient {
    let client_id = ClientId::new("YOUR_CLIENT_ID".to_string());
    let client_secret = ClientSecret::new("YOUR_CLIENT_SECRET".to_string());
    let auth_url = AuthUrl::new("https://api.notion.com/v1/oauth/authorize".to_string())
        .expect("Invalid authorization endpoint URL"); 
    let token_url = TokenUrl::new("https://api.notion.com/v1/oauth/token".to_string())
        .expect("Invalid token endpoint URL");
    let redirect_url = RedirectUrl::new("http://localhost:8080/callback".to_string())
        .expect("Invalid redirect URL");

    let client= BasicClient::new(client_id)
    .set_client_secret(client_secret)
    .set_auth_uri(auth_url)
    .set_token_uri(token_url)
    .set_redirect_uri(redirect_url);

    client
    
}

pub async fn authenticate_with_notion(){
    let client = build_oauth_client();
    let (auth_url, _csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .url();
    
    if webbrowser::open(auth_url.as_str()).is_ok() {
        println!("Opened browser for authentication. Please complete the process.");
    } else {
        println!("Please open the following URL in your browser to authenticate: {}", auth_url);
    }
}