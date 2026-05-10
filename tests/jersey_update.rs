use pila::AppState;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
async fn test_jersey_post_returns_updated_leaderboard_with_oob() {
    dotenvy::dotenv().ok();
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let user_id = Uuid::new_v4();
    let token = format!("test-token-{}", uuid::Uuid::new_v4());

    sqlx::query!(
        "INSERT INTO users (id, name, token, jersey_preset) VALUES ($1, $2, $3, $4)",
        user_id,
        "Test User",
        token,
        "classic"
    )
    .execute(&pool)
    .await
    .unwrap();

    let jerseys = pila::jersey::load();
    assert!(jerseys.contains_key("classic"), "classic jersey preset must exist");
    assert!(
        jerseys.contains_key("brasilien"),
        "brasilien jersey preset must exist"
    );

    let updated_user = sqlx::query!("SELECT jersey_preset FROM users WHERE id = $1", user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        updated_user.jersey_preset, "classic",
        "Initial jersey should be classic"
    );

    let _state = AppState {
        jerseys: jerseys.clone(),
        news: pila::news::NewsCache::from_env(),
        repos: pila::repo::Repos::from_pool(pool.clone()),
        translations: std::collections::HashMap::new(),
    };

    let new_jersey = "brasilien";
    sqlx::query!(
        "UPDATE users SET jersey_preset = $1 WHERE id = $2",
        new_jersey,
        user_id
    )
    .execute(&pool)
    .await
    .unwrap();

    let updated_user = sqlx::query!("SELECT jersey_preset FROM users WHERE id = $1", user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        updated_user.jersey_preset, "brasilien",
        "Jersey should be updated to brasilien"
    );

    let leaderboard = sqlx::query!(
        r#"
        SELECT u.name, u.jersey_preset
        FROM users u
        WHERE u.id = $1
        "#,
        user_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(leaderboard.name, "Test User");
    assert_eq!(leaderboard.jersey_preset, "brasilien");
}
