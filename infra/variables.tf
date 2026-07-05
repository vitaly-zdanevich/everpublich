variable "tenancy_ocid" {
  type        = string
  description = "OCI tenancy OCID."
}

variable "user_ocid" {
  type        = string
  description = "OCI user OCID for Terraform API calls."
}

variable "fingerprint" {
  type        = string
  description = "Fingerprint of the OCI API signing key."
}

variable "private_key_path" {
  type        = string
  description = "Path to the OCI API signing private key."
}

variable "private_key_password" {
  type        = string
  description = "Optional password for the OCI API signing private key."
  sensitive   = true
  default     = ""
}

variable "region" {
  type        = string
  description = "OCI home region where Always Free resources are available."
}

variable "compartment_ocid" {
  type        = string
  description = "OCI compartment OCID for the VM and network. The tenancy OCID can be used if you deploy into the root compartment."
}

variable "project_name" {
  type        = string
  description = "Name prefix for OCI resources."
  default     = "everpublich"
}

variable "repository_url" {
  type        = string
  description = "Git repository cloned onto the VM for in-place ARM builds."
  default     = "https://github.com/vitaly-zdanevich/everpublich.git"
}

variable "base_domain" {
  type        = string
  description = "Root domain for per-user subdomains. Until the TLD is bought, test with the VM public IP or local hosts entries."
  default     = "everpublich.xyz"
}

variable "instance_image_ocid" {
  type        = string
  description = "OCID of an Always Free eligible Ubuntu AArch64 image in your OCI home region."

  validation {
    condition     = length(var.instance_image_ocid) > 0
    error_message = "Set instance_image_ocid to an Ubuntu AArch64 image OCID from your OCI region."
  }
}

variable "availability_domain_index" {
  type        = number
  description = "Zero-based availability-domain index. Try another value if OCI reports out of host capacity."
  default     = 0
}

variable "instance_ocpus" {
  type        = number
  description = "Ampere A1 OCPUs. The Always Free total is currently 2 OCPUs."
  default     = 2

  validation {
    condition     = var.instance_ocpus > 0 && var.instance_ocpus <= 2
    error_message = "Use 1 or 2 OCPUs to stay inside OCI Always Free Ampere A1 limits."
  }
}

variable "instance_memory_gb" {
  type        = number
  description = "Ampere A1 memory in GB. The Always Free total is currently 12 GB."
  default     = 12

  validation {
    condition     = var.instance_memory_gb >= 1 && var.instance_memory_gb <= 12
    error_message = "Use 1 through 12 GB to stay inside OCI Always Free Ampere A1 limits."
  }
}

variable "boot_volume_size_gb" {
  type        = number
  description = "Boot volume size. OCI Always Free gives 200 GB total block storage, including boot volumes."
  default     = 200

  validation {
    condition     = var.boot_volume_size_gb >= 50 && var.boot_volume_size_gb <= 200
    error_message = "Use 50 through 200 GB for the boot volume."
  }
}

variable "ssh_public_key_path" {
  type        = string
  description = "Path to the public key installed for SSH access to the VM."
  default     = "~/.ssh/id_ed25519.pub"
}

variable "ssh_user" {
  type        = string
  description = "Default SSH user for the chosen image. Ubuntu images normally use ubuntu."
  default     = "ubuntu"
}

variable "allowed_ssh_cidrs" {
  type        = list(string)
  description = "CIDR blocks allowed to SSH to the VM. Tighten this after the first deploy."
  default     = ["0.0.0.0/0"]
}

variable "vcn_cidr" {
  type        = string
  description = "CIDR block for the Everpublich VCN."
  default     = "10.42.0.0/16"
}

variable "public_subnet_cidr" {
  type        = string
  description = "CIDR block for the public subnet."
  default     = "10.42.1.0/24"
}

variable "vcn_dns_label" {
  type        = string
  description = "DNS label for the OCI VCN. Use letters and numbers only."
  default     = "everpublich"
}

variable "subnet_dns_label" {
  type        = string
  description = "DNS label for the OCI subnet. Use letters and numbers only."
  default     = "public"
}

variable "instance_hostname_label" {
  type        = string
  description = "Hostname label for the VM VNIC. Use letters and numbers only."
  default     = "everpublich"
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
  description = "Install the configured AArch64 Zola release during cloud-init."
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

variable "evernote_appimage_repository" {
  type        = string
  description = "GitHub owner/repo that publishes Evernote AppImage releases."
  default     = "vitaly-zdanevich/evernote-linux-repackage"
}

variable "evernote_appimage_asset_regex" {
  type        = string
  description = "Regex used against release asset names. The default selects the normal AArch64 AppImage, not the black-theme variant."
  default     = "^Evernote-.*-aarch64\\.AppImage$"
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
