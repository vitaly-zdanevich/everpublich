output "public_ip" {
  description = "Public IPv4 address of the Everpublich ARM VM."
  value       = oci_core_instance.app.public_ip
}

output "landing_url" {
  description = "Temporary HTTP URL before DNS and HTTPS are configured."
  value       = "http://${oci_core_instance.app.public_ip}/"
}

output "ssh_command" {
  description = "SSH command for the VM. Add -i if your key is not the default SSH key."
  value       = "ssh ${var.ssh_user}@${oci_core_instance.app.public_ip}"
}

output "dns_records_to_create" {
  description = "Create these records at the DNS provider after buying the TLD."
  value = [
    "A ${var.base_domain} -> ${oci_core_instance.app.public_ip}",
    "A *.${var.base_domain} -> ${oci_core_instance.app.public_ip}"
  ]
}

output "sqlite_database_path" {
  description = "SQLite application database path on the VM."
  value       = "/var/lib/everpublich/db/everpublich.sqlite"
}

output "generated_sites_path" {
  description = "Generated static sites root on the VM."
  value       = "/var/www/everpublich/sites"
}

output "evernote_config_path" {
  description = "Official Evernote client config/cache path when run as the everpublich Linux user."
  value       = "/var/lib/everpublich/.config/Evernote"
}

output "evernote_appimage_path" {
  description = "Installed Evernote AppImage path on the VM."
  value       = "/opt/everpublich/app/Evernote.AppImage"
}

output "evernote_ssh_x_command" {
  description = "SSH X forwarding command for interactive Evernote login."
  value       = "ssh -Y ${var.ssh_user}@${oci_core_instance.app.public_ip}"
}
