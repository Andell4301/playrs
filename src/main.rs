use anyhow::{Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use playrs::{GooglePlayApi, MessageJsonExt, constants::PatchFormat};
use std::{fs, path::PathBuf};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(flatten)]
    auth: AuthArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Download(DownloadArgs),
    AppDetails(AppDetailsArgs),
    AuthInfo,
}

#[derive(Debug, Args)]
struct AuthArgs {
    #[arg(long)]
    gsf_id: Option<String>,
    #[arg(long, requires = "gsf_id")]
    auth_token: Option<String>,
    #[arg(long)]
    aas_token: Option<String>,
    #[arg(long)]
    oauth_login_token: Option<String>,
    #[arg(long)]
    email: Option<String>,
    #[arg(long)]
    device_name: Option<String>,
    #[arg(long)]
    device_locale: Option<String>,
}

#[derive(Debug, Args)]
struct DownloadArgs {
    #[arg(long)]
    package_name: String,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    version_code: Option<i64>,
    #[arg(long)]
    offer_type: Option<i32>,
    #[arg(long)]
    certificate_hash: Option<String>,
    #[arg(long)]
    split_module: Option<String>,
    #[arg(long)]
    installed_version_code: Option<i64>,
    #[arg(long, value_enum, default_value_t = PatchFormat::GzippedBsdiff)]
    patch_format: PatchFormat,
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true")]
    use_xapk: bool,
    #[arg(long)]
    custom_apk_name: Option<String>,
    #[arg(long, default_value_t = false, num_args = 0..=1, default_missing_value = "true")]
    include_dex: bool,
}

#[derive(Debug, Args)]
struct AppDetailsArgs {
    #[arg(long)]
    package_name: String,
    #[arg(long, default_value = "app_details.json")]
    output_file: PathBuf,
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true")]
    pretty: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("playrs=debug".parse()?)).init();

    let cli = Cli::parse();
    let mut api = build_api(&cli.auth)?;
    api.setup(false).await?;

    match cli.command {
        Command::Download(args) => {
            let (version_code, offer_type) = resolve_version_code_and_offer_type(&api, &args).await?;

            api.download_app(
                &args.package_name,
                &args.output_dir,
                Some(version_code),
                Some(offer_type),
                args.certificate_hash.as_deref(),
                args.split_module.as_deref(),
                args.installed_version_code,
                args.patch_format,
                args.use_xapk,
                args.custom_apk_name,
                args.include_dex,
            )
            .await?;
        }
        Command::AppDetails(args) => {
            write_app_details_json(&api, &args).await?;
        }
        Command::AuthInfo => {
            print_auth_info(&api)?;
        }
    }

    Ok(())
}

fn build_api(auth: &AuthArgs) -> Result<GooglePlayApi> {
    let locale = auth.device_locale.as_deref();

    match (auth.gsf_id.as_ref(), auth.aas_token.as_ref(), auth.oauth_login_token.as_ref()) {
        (Some(gsf_id), None, None) => Ok(GooglePlayApi::new_from_gsf_id(
            gsf_id.clone(),
            auth.auth_token.clone().ok_or_else(|| anyhow!("--auth-token is required when using --gsf-id"))?,
            auth.device_name.clone(),
            locale,
        )?),

        (None, Some(aas_token), None) => Ok(GooglePlayApi::new_from_aas_token(
            aas_token.clone(),
            auth.email.clone().ok_or_else(|| anyhow!("--email is required when using --aas-token"))?,
            auth.device_name.clone(),
            locale,
        )?),

        (None, None, Some(oauth_login_token)) => Ok(GooglePlayApi::new_from_oauth_login_token(
            oauth_login_token.clone(),
            auth.email.clone().ok_or_else(|| anyhow!("--email is required when using --oauth-login-token"))?,
            auth.device_name.clone(),
            locale,
        )?),

        (None, None, None) => {
            bail!("authorization is required; please provide one of the following: --gsf-id, --aas-token, or --oauth-login-token")
        }

        _ => {
            bail!("please choose only one auth method: --gsf-id, --aas-token, or --oauth-login-token")
        }
    }
}

// TODO: It may not be necessary to fetch app details if the only missing piece is offer type?
async fn resolve_version_code_and_offer_type(api: &GooglePlayApi, args: &DownloadArgs) -> Result<(i64, i32)> {
    if let (Some(version_code), Some(offer_type)) = (args.version_code, args.offer_type) {
        return Ok((version_code, offer_type));
    }

    let details = api.get_app_details_by_package_name(&args.package_name).await?;
    let item = details.item.ok_or_else(|| anyhow!("missing item in app details for package {}", args.package_name))?;

    let fetched_version_code = item.details.and_then(|d| d.app_details).and_then(|a| a.version_code);
    let fetched_offer_type = item.offer.into_iter().next().and_then(|offer| offer.offer_type);

    let version_code = match args.version_code {
        Some(v) => v,
        None => fetched_version_code
            .ok_or_else(|| anyhow!("version code was not provided and could not be found in app details for {}", args.package_name))?,
    };

    let offer_type = args.offer_type.or(fetched_offer_type).unwrap_or(1);
    Ok((version_code, offer_type))
}

async fn write_app_details_json(api: &GooglePlayApi, args: &AppDetailsArgs) -> Result<()> {
    let details = api.get_app_details_by_package_name(&args.package_name).await?;
    let json_value = details.to_json_value()?;
    let json = if args.pretty { serde_json::to_string_pretty(&json_value)? } else { serde_json::to_string(&json_value)? };

    if let Some(parent) = args.output_file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(&args.output_file, json)?;
    info!("Wrote app details JSON to {}", args.output_file.display());
    Ok(())
}

fn print_auth_info(api: &GooglePlayApi) -> Result<()> {
    let auth_data = api.get_auth_data();
    let gsf_id = auth_data.get_gsf_id().ok_or_else(|| anyhow!("setup completed, but GSF ID is missing"))?;
    let auth_token = auth_data.get_auth_token().ok_or_else(|| anyhow!("setup completed, but Auth Token is missing"))?;
    println!("GSF ID: {gsf_id}");
    println!("Auth Token: {auth_token}");
    Ok(())
}
