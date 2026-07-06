variable "aws_region" {
  type        = string
  description = "AWS region for the EC2 origin and EBS volumes."
  default     = "us-east-1"
}

variable "availability_zone" {
  type        = string
  description = "Optional availability zone for the EC2 instance and Btrfs EBS volume. Empty means the first available AZ in aws_region."
  default     = ""
}

variable "project_name" {
  type        = string
  description = "Name prefix for AWS resources."
  default     = "everpublich"
}

variable "repository_url" {
  type        = string
  description = "Git repository cloned onto the VM for in-place builds."
  default     = "https://github.com/vitaly-zdanevich/everpublich.git"
}

variable "base_domain" {
  type        = string
  description = "Root domain for per-user subdomains. Until CloudFront aliases are configured, test generated sites with the CloudFront URL."
  default     = "everpublich.my"
}

variable "instance_type" {
  type        = string
  description = "EC2 instance type. m7i-flex.large gives 2 vCPU and 8 GiB RAM and is listed by AWS as Free Plan eligible."
  default     = "m7i-flex.large"
}

variable "instance_architecture" {
  type        = string
  description = "CPU architecture for the selected instance type and Ubuntu AMI."
  default     = "x86_64"

  validation {
    condition     = contains(["x86_64", "arm64"], var.instance_architecture)
    error_message = "Use x86_64 or arm64."
  }
}

variable "ubuntu_release" {
  type        = string
  description = "Ubuntu LTS release used for the EC2 AMI lookup."
  default     = "24.04"
}

variable "root_volume_size_gb" {
  type        = number
  description = "Root EBS volume size in GiB. Keep root + data volumes at or below 30 GiB to fit the classic EBS Free Tier allowance."
  default     = 10

  validation {
    condition     = var.root_volume_size_gb >= 8
    error_message = "Use at least 8 GiB for the Ubuntu root volume."
  }
}

variable "associate_public_ipv4" {
  type        = bool
  description = "Assign a public IPv4 address to the EC2 instance. AWS charges hourly for public IPv4, so the default is IPv6-only."
  default     = false
}

variable "enable_ipv6" {
  type        = bool
  description = "Assign Amazon-provided IPv6 to the VPC, subnet, and EC2 instance."
  default     = true
}

variable "data_volume_size_gb" {
  type        = number
  description = "Compressed Btrfs data EBS volume size in GiB for Evernote cache, SQLite, generated sites, and build checkout."
  default     = 20

  validation {
    condition     = var.data_volume_size_gb >= 1
    error_message = "Use at least 1 GiB for the Everpublich data volume."
  }
}

variable "btrfs_zstd_level" {
  type        = number
  description = "Btrfs zstd compression level for the data volume. 15 is the maximum supported level."
  default     = 15

  validation {
    condition     = var.btrfs_zstd_level >= 1 && var.btrfs_zstd_level <= 15
    error_message = "Use a Btrfs zstd compression level from 1 through 15."
  }
}

variable "ssh_public_key_path" {
  type        = string
  description = "Path to the public key installed for SSH access to the EC2 instance."
  default     = "~/.ssh/id_ed25519.pub"
}

variable "ssh_key_pair_name" {
  type        = string
  description = "AWS EC2 key pair name created from ssh_public_key_path."
  default     = "everpublich-ssh"
}

variable "ssh_user" {
  type        = string
  description = "Default SSH user for the chosen Ubuntu image."
  default     = "ubuntu"
}

variable "allowed_ssh_cidrs" {
  type        = list(string)
  description = "IPv4 CIDR blocks allowed to SSH to the EC2 instance. Empty by default because public IPv4 is disabled."
  default     = []
}

variable "allowed_ssh_ipv6_cidrs" {
  type        = list(string)
  description = "IPv6 CIDR blocks allowed to SSH to the EC2 instance. Tighten this after the first deploy."
  default     = ["::/0"]
}

variable "vpc_cidr" {
  type        = string
  description = "CIDR block for the Everpublich VPC."
  default     = "10.42.0.0/16"
}

variable "public_subnet_cidr" {
  type        = string
  description = "CIDR block for the public subnet."
  default     = "10.42.1.0/24"
}

variable "zola_version" {
  type        = string
  description = "Zola version installed by cloud-init when install_zola_on_boot is true."
  default     = "0.19.2"
}

variable "install_rustup_on_boot" {
  type        = bool
  description = "Install a minimal Rust toolchain under the everpublich Linux user during cloud-init."
  default     = true
}

variable "install_zola_on_boot" {
  type        = bool
  description = "Install the configured Zola release during cloud-init."
  default     = true
}

variable "install_evernote_gui_deps" {
  type        = bool
  description = "Install common GUI/keyring libraries needed by the official Evernote Linux client."
  default     = true
}

variable "install_evernote_appimage_on_boot" {
  type        = bool
  description = "Download and install the latest Evernote AppImage during cloud-init."
  default     = true
}

variable "run_evernote_on_boot" {
  type        = bool
  description = "Start the Evernote AppImage systemd service after cloud-init installs it."
  default     = true
}

variable "install_cloudwatch_agent_on_boot" {
  type        = bool
  description = "Install and start the Amazon CloudWatch Agent for EC2 RAM and disk metrics during cloud-init."
  default     = true
}

variable "evernote_appimage_repository" {
  type        = string
  description = "GitHub owner/repo that publishes Evernote AppImage releases."
  default     = "vitaly-zdanevich/evernote-linux-repackage"
}

variable "evernote_appimage_asset_regex" {
  type        = string
  description = "Regex used against release asset names. The default selects the normal x86_64 AppImage for m7i-flex.large."
  default     = "^Evernote-v[0-9][0-9.]*-[0-9]+-x86_64\\.AppImage$"
}

variable "create_cloudfront_distribution" {
  type        = bool
  description = "Create a CloudFront distribution in front of the private S3 generated-sites bucket."
  default     = true
}

variable "sites_bucket_name" {
  type        = string
  description = "Optional globally unique S3 bucket name for generated static sites. Empty uses project-name, account ID, and region."
  default     = ""
}

variable "cloudfront_price_class" {
  type        = string
  description = "CloudFront edge location price class."
  default     = "PriceClass_100"

  validation {
    condition     = contains(["PriceClass_100", "PriceClass_200", "PriceClass_All"], var.cloudfront_price_class)
    error_message = "Use PriceClass_100, PriceClass_200, or PriceClass_All."
  }
}

variable "cloudfront_aliases" {
  type        = list(string)
  description = "Optional custom domains for CloudFront, for example everpublich.my and *.everpublich.my. Requires an ACM certificate in us-east-1."
  default     = []
}

variable "cloudfront_acm_certificate_arn" {
  type        = string
  description = "ACM certificate ARN in us-east-1 for cloudfront_aliases. Leave empty when using the default cloudfront.net domain."
  default     = ""

  validation {
    condition     = length(var.cloudfront_aliases) == 0 || length(var.cloudfront_acm_certificate_arn) > 0
    error_message = "Set cloudfront_acm_certificate_arn when cloudfront_aliases is not empty."
  }
}

variable "cloudfront_wait_for_deployment" {
  type        = bool
  description = "Wait for CloudFront deployment completion during terraform apply."
  default     = false
}

variable "create_cloudwatch_dashboard" {
  type        = bool
  description = "Create an operations CloudWatch dashboard for Everpublich metrics, EC2, S3, and CloudFront."
  default     = true
}

variable "cloudfront_free_tier_bytes_per_month" {
  type        = number
  description = "Monthly CloudFront transfer allowance used to draw the dashboard free-tier line. Default is 100 GB/month."
  default     = 100000000000

  validation {
    condition     = var.cloudfront_free_tier_bytes_per_month > 0
    error_message = "CloudFront free-tier bytes per month must be positive."
  }
}

variable "support_email" {
  type        = string
  description = "Support email shown by app/admin surfaces."
  default     = "zdanevich.vitaly@ya.ru"
}

variable "support_telegram" {
  type        = string
  description = "Support Telegram link shown by app/admin surfaces."
  default     = "https://t.me/vitaly_zdanevich"
}

variable "support_tickets" {
  type        = string
  description = "Support issue tracker link shown by app/admin surfaces."
  default     = "https://github.com/vitaly-zdanevich/everpublich/issues"
}

variable "genius_token" {
  type        = string
  description = "Optional Genius Client Access Token used to resolve genius.com links to YouTube and lyrics embeds."
  default     = ""
  sensitive   = true
}
