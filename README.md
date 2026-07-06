# Everpublich: sync Evernote notebook to a static blog, like postach.io and notesrss.com

[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=coverage)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Bugs](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=bugs)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Vulnerabilities](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=vulnerabilities)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Code Smells](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=code_smells)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Duplicated Lines](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=duplicated_lines_density)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Maintainability](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=sqale_rating)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Reliability](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=reliability_rating)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Security](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=security_rating)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Lines of Code](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=ncloc)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)
[![Technical Debt](https://sonarcloud.io/api/project_badges/measure?project=vitaly-zdanevich_everpublich&metric=sqale_index)](https://sonarcloud.io/summary/new_code?id=vitaly-zdanevich_everpublich)

Everpublich is a free MVP test pilot that turns an [Evernote](https://evernote.com/) notebook into a fast static [Zola](https://www.getzola.org/) blog or website. It aims to be a better version of [Postach.io](https://postach.io/) and [NotesRSS](https://notesrss.com/): RSS, tags, static search, calendar, podcast feed, media playback, expanded links-to-widgets, backup value, and generated static websites served from S3 through CloudFront or mirrored to GitHub.

Free during the test stage.

I use Evernote from 2009 and love it.

## Product

The current MVP avoids Evernote OAuth because Evernote no longer issues legacy API keys to new third-party services. Users share a notebook as read-only to the Everpublich service Evernote account. The official Evernote Linux client syncs that account on the EC2 builder, and Everpublich reads the client cache read-only to rebuild websites once per hour.

User and site settings live in SQLite, not DynamoDB. Generated static websites and copied media are synced to a private S3 bucket and served through CloudFront. The default home page shows full posts, with a SQLite preference to switch the home page to titles only.

The public landing page can still be published to [GitHub Pages](https://docs.github.com/en/pages) from GitHub Actions. For the free MVP, it asks users to share a notebook read-only with `everpublich@proton.me`; Everpublich will email the generated website link after the first sync.

## Architecture

```mermaid
flowchart TD
	User[User browser] --> Landing[GitHub Pages landing and admin shell]
	Landing --> VmApi[Rust API on EC2 builder]
	User --> Share[Share notebook read-only]
	Share --> ServiceAccount[Everpublich service Evernote account]
	ServiceAccount --> EvernoteClient[Official Evernote Linux client]
	EvernoteClient --> EvernoteCache[(Evernote SQLite/cache files)]
	VmApi --> Users[(SQLite users/settings)]
	Timer[systemd hourly timer] --> Builder[Rust full-regeneration worker]
	Users --> Builder
	EvernoteCache --> Builder
	Builder --> Zola[Zola source tree]
	Zola --> S3[(Private S3 generated-sites bucket)]
	S3 --> CloudFront[CloudFront CDN]
	CloudFront --> Site[User static website]
	Builder --> GitRepo[Optional GitHub repo backup]
```

## Evernote access

The official API path is blocked for new Evernote developers today, so the MVP uses a service account and shared notebooks:

- The user creates or chooses a notebook intended for publishing.
- The user shares that notebook read-only to the Everpublich service Evernote account.
- The official Evernote client syncs the shared notebook on the VM.
- The Rust parser reads the local Evernote cache and SQLite files read-only.

Local inspection of the official Linux client shows SQLite databases under `~/.config/Evernote/conduit-storage/.../*.sql`, with note, notebook, tag, attachment, and offline search tables. Note bodies and resource data are also cached under `conduit-fs`. This is a private client storage format, so the parser must be defensive, tested against snapshots, and pinned to a known client version.

The parser must never modify Evernote cache files. For reliable reads, it should copy SQLite databases to a temporary snapshot before querying, especially if the desktop client is running and using WAL files.

## Website features

- Full static regeneration once per hour.
- [Zola](https://www.getzola.org/) site generation with `minify_html = true`.
- Use any [Zola theme](https://www.getzola.org/themes/) or add custom CSS.
- I can develop a custom visual theme for you.
- Full posts on the main page by default, with a setting for titles only.
- Static search by default, plus optional Google search.
- RSS and `sitemap.xml`.
- Podcast XML feed from notes tagged `podcast`.
- Tags: every Evernote tag gets a page.
- Notes tagged `page` become dedicated website pages.
- A note titled `everpublich:about` becomes the About page.
- A note titled `everpublish:Something` becomes a dedicated page named `Something`.
- A note titled `everpublich:#tag` adds a top navigation link to that tag; the note content becomes the link tooltip.
- A note titled `everpublich:config` can set notebook options like `widgets: off` and can embed fenced `css` or `js` code blocks into the generated website.
- If an About note references about.me, the intended behavior is to reuse text, image, and links from that profile and link back to it.
- Images, audio, video, and attachments from Evernote notes are copied to the static site.
- Audio and video are playable in the browser.
- Internal Evernote note links become relative website links.
- Evernote formatting is preserved as HTML, including fonts, sizes, colors, and tables.
- Optional Google Analytics and Yandex Metrica.
- Mobile-friendly design with black dark mode via `prefers-color-scheme`.
- Offline support in the browser.
- Minimal JavaScript, static HTML, minified output, and CloudFront gzip/Brotli compression.
- Backup value: the generated site and optional GitHub repository become another copy of the Evernote notebook.

## Widget expansion

If a note contains a bare supported URL, Everpublich can expand it into a widget. Current providers:

- YouTube
- Instagram
- Pinterest
- Spotify
- Genius
- SoundCloud
- Apple Podcasts
- Vimeo
- Rumble
- Dailymotion
- Bilibili
- Odysee
- Yandex Music
- Steam
- VK playlists
- Mastodon posts and static profile cards

Good extra widget candidates:

- Bandcamp for music and albums
- TikTok for short videos
- Twitch for clips and videos
- Mixcloud for DJ/radio sets
- Internet Archive for books, audio, and video
- GitHub Gist and CodePen for code
- Figma embeds for design files
- Google Maps for places
- Reddit, Bluesky, and Telegram public posts

## GitHub backup

The admin panel can connect GitHub OAuth and switch backup repository visibility between private and public. Private is the safer default. Git is useful because it stores all versions, but if you accidentally publish something private, you also need to fix git history. You can write to Vitaly for help.

## Subdomains

Automatic per-user subdomains are feasible through CloudFront. After buying the TLD and creating an ACM certificate in `us-east-1`, set `cloudfront_aliases` and create DNS records at any registrar or DNS provider:

- `CNAME *.everpublich.xyz -> CLOUDFRONT_DOMAIN`
- `ALIAS/ANAME everpublich.xyz -> CLOUDFRONT_DOMAIN`, if you decide to move the root domain from GitHub Pages later

Registering the TLD outside AWS can be cheaper than using Route 53 as a registrar. The landing page stays on GitHub Pages. Until the domain is bought, test generated sites with the CloudFront URL, for example `https://CLOUDFRONT_DOMAIN/demo/`.

CloudFront can automatically compress eligible objects with gzip and Brotli. Everpublich still asks Zola to minify HTML.

## Similar products

- [Postach.io](https://postach.io/) - Evernote-powered blogging platform.
- [NotesRSS](https://notesrss.com/) - Evernote blog service with free blog positioning and CDN hosting.
- [Blot](https://blot.im/) - static sites from a folder, commonly Dropbox or Git.
- [Super](https://super.so/) - websites from Notion pages.
- [Potion](https://potion.so/) - Notion website builder.
- [Feather](https://feather.so/) - Notion-to-blog publishing.

Public market notes from the online check:

- NotesRSS sells simplicity: write in Evernote and publish with a tag.
- NotesRSS also highlights CDN hosting, which supports adding Cloudflare later when the domain exists.
- Evernote API access is the largest platform risk, so the service-account plus desktop-cache path is the practical MVP.
- Evernote free-plan reductions create demand for backup and export-oriented tools.
- I found limited current public review material for Postach.io and NotesRSS, so the product should include a fast feedback loop and author support links from day one.

## Startup feedback

A $5/month SaaS can work if the product solves backup, publishing, and ownership better than a simple blog service. The risk is Evernote platform access and the smaller Evernote power-user market. The strongest MVP angle is not “blogging only”; it is “publish and back up an Evernote notebook as a fast static website”.

Feature ideas:

- Import from Evernote export files (`.enex`) for users who do not want to share a notebook.
- Custom domain setup wizard.
- Search engine indexing diagnostics.
- Private site mode with password or signed URLs.
- Email newsletter from RSS.
- Webmention support.
- Markdown export and ZIP backup.
- Broken-link checker.
- AI-generated summaries and tag cleanup, optional and transparent.
- Paid custom theme setup.

Related startup ideas:

- Notion-to-static-site with Git backup and clean export.
- Google Keep export-to-blog, but Google Keep has weaker API/export ergonomics.
- Obsidian vault to static site with media, backlinks, and private sections.
- “Personal knowledge backup monitor” that checks Evernote, Notion, Google Drive, GitHub, and Telegram exports.
- Static podcast generator from folders, notebooks, or YouTube playlists.
- Hosted “about me” page that syncs from existing profiles and notes.
- Small-business knowledge base from Notion, Evernote, or Google Docs to static site.
- Personal archive search across Evernote exports, Telegram exports, browser bookmarks, and local files.

Notion is worth supporting later because the market is larger and website builders around Notion already proved demand. Evernote is a better first niche for you because you have long-term product intuition and related projects.

## Future plans

If this free MVP gets more than 100 GitHub stars:

- Other static website generators.
- From-Evernote-to-WordPress sync support.
- Sync to Telegram channel.
- Automatic backend translation to different languages.
- More import sources, including Notion and Obsidian.

You can also send your ideas.

## Other Evernote projects by Vitaly

- [bot_telegram_evernote](https://gitlab.com/vitaly-zdanevich/bot_telegram_evernote) - Telegram bot for searching Evernote notes and saving Telegram attachments into Evernote.
- [pinterest-saves-to-evernote](https://github.com/vitaly-zdanevich/pinterest-saves-to-evernote) - saves Pinterest content to Evernote.
- [yandex-music-likes-to-evernote](https://github.com/vitaly-zdanevich/yandex-music-likes-to-evernote) - syncs Yandex Music likes to Evernote.
- [geeknote](https://github.com/vitaly-zdanevich/geeknote) - Evernote CLI.
- [reeknote](https://gitlab.com/vitaly-zdanevich/reeknote) - Rust rewrite of an Evernote CLI.

Related project:

- [telegram_channel_to_static_website](https://github.com/vitaly-zdanevich/telegram_channel_to_static_website) - public Telegram channel to static Zola website. Everpublich takes visual and product ideas from it, including the calendar.

## Local development

```sh
cargo test
cargo run --bin everpublich-cli -- mock-site --output build/mock-site
zola --root build/mock-site serve
```

The end-to-end HTML test runs `zola build`, so install [Zola](https://www.getzola.org/documentation/getting-started/installation/) before `cargo test --all-targets`.

## AWS EC2 builder deployment

The Terraform in `infra/` provisions one EC2 builder, a VPC, a public IPv6 subnet, a private S3 bucket for generated static websites, CloudFront with Origin Access Control, SQLite, an hourly systemd timer, and directories for the official Evernote client cache and generated websites.

The default instance is `m7i-flex.large` with 2 vCPU and 8 GiB RAM. Public IPv4 is disabled by default because AWS charges hourly for it. IPv6 is enabled by default. The repo keeps the classic 30 GiB EBS allowance split as 10 GiB root plus a 20 GiB Btrfs data volume mounted with `compress-force=zstd:15`.

AWS currently lists CloudFront Free plan allowances as 100 GB data transfer, 1M requests, and 5 GB included S3 storage per month. Generated sites should stay inside that during the MVP if media usage is modest.

Create `infra/terraform.tfvars` from `infra/terraform.tfvars.example`, review the region and SSH key path, then:

```sh
./scripts/deploy.sh
```

If bootstrap cannot download GitHub/rustup/AppImage assets over IPv6, temporarily set `associate_public_ipv4 = true`, apply, complete bootstrap, then switch it back to `false` to avoid the public IPv4 hourly charge.

Scripts:

- `scripts/deploy.sh` runs Terraform for the AWS EC2/S3/CloudFront stack.
- `scripts/update-code.sh` SSHes into the EC2 builder, pulls the repo, builds `everpublich-cli`, installs it, and starts the sync service.
- `scripts/show-logs.sh` reads `journalctl` logs for `everpublich-sync.service` over SSH.
- `scripts/build_pages.py` builds the GitHub Pages artifact into `dist/pages`.

Evernote AppImage login is interactive. After deployment, connect with SSH X forwarding, run the helper in your forwarded X session, log in once, then restart the background service:

```sh
ssh -Y ubuntu@EC2_HOST
/opt/everpublich/bin/evernote-run-ssh-x
sudo systemctl start evernote-client.service
```

Example:

```sh
EVERPUBLICH_SSH_HOST=EC2_HOST ./scripts/update-code.sh
EVERPUBLICH_SSH_HOST=EC2_HOST ./scripts/show-logs.sh
```

CI publishes GitHub Pages on pushes to `main`. Optional repository variables:

- `EVERPUBLICH_PAGES_API_BASE_URL` - VM API base URL used by the connect/admin browser calls.

CI also generates `coverage/lcov.info` with `cargo-llvm-cov`; `sonar-project.properties` points SonarCloud at that report.

## Documentation links

- [Evernote developer documentation](https://dev.evernote.com/doc/)
- [Evernote notebook sharing](https://dev.evernote.com/doc/articles/notebook_sharing.php)
- [Zola documentation](https://www.getzola.org/documentation/getting-started/overview/)
- [Zola themes](https://www.getzola.org/themes/)
- [Amazon EC2 instance types](https://aws.amazon.com/ec2/instance-types/)
- [Amazon EBS pricing](https://aws.amazon.com/ebs/pricing/)
- [Amazon S3 pricing](https://aws.amazon.com/s3/pricing/)
- [Amazon CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/)
- [CloudFront compression](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/ServingCompressedFiles.html)
- [CloudFront Origin Access Control for S3](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/private-content-restricting-access-to-s3.html)
- [Terraform AWS provider](https://registry.terraform.io/providers/hashicorp/aws/latest/docs)
- [SQLite documentation](https://www.sqlite.org/docs.html)
- [Btrfs documentation](https://btrfs.readthedocs.io/)

## Support

This service is a free MVP test pilot. Support is available from the author, Vitaly Zdanevich:

- Telegram: [@vitaly_zdanevich](https://t.me/vitaly_zdanevich)
- Email: [zdanevich.vitaly@ya.ru](mailto:zdanevich.vitaly@ya.ru)
- Tickets: [GitHub issues](https://github.com/vitaly-zdanevich/everpublich/issues)
