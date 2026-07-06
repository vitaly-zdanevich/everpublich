output "public_ip" {
  description = "Public IPv4 address of the Everpublich EC2 origin."
  value       = aws_instance.app.public_ip != "" ? aws_instance.app.public_ip : null
}

output "public_ipv6" {
  description = "Public IPv6 address of the Everpublich EC2 origin."
  value       = length(aws_instance.app.ipv6_addresses) > 0 ? aws_instance.app.ipv6_addresses[0] : null
}

output "public_dns" {
  description = "Public DNS name of the Everpublich EC2 origin."
  value       = aws_instance.app.public_dns != "" ? aws_instance.app.public_dns : null
}

output "cloudfront_url" {
  description = "CloudFront URL for generated user websites."
  value       = length(aws_cloudfront_distribution.sites) > 0 ? "https://${aws_cloudfront_distribution.sites[0].domain_name}/" : null
}

output "cloudfront_note" {
  description = "CloudFront serving note."
  value       = length(aws_cloudfront_distribution.sites) > 0 ? "CloudFront serves generated user websites from private S3. The landing page remains on GitHub Pages." : "CloudFront is disabled; generated websites remain in S3."
}

output "ssh_command" {
  description = "SSH command for the EC2 instance. Add -i if your key is not the default SSH key."
  value       = aws_instance.app.public_dns != "" ? "ssh ${var.ssh_user}@${aws_instance.app.public_dns}" : "ssh ${var.ssh_user}@${aws_instance.app.ipv6_addresses[0]}"
}

output "dns_records_to_create" {
  description = "Create these records at the DNS provider after buying the TLD and setting cloudfront_aliases plus an ACM certificate."
  value = length(aws_cloudfront_distribution.sites) > 0 ? [
    "CNAME *.${var.base_domain} -> ${aws_cloudfront_distribution.sites[0].domain_name}",
    "Keep ${var.base_domain} on GitHub Pages for the landing page."
    ] : [
    "CloudFront disabled; generated websites stay private in S3."
  ]
}

output "sqlite_database_path" {
  description = "SQLite application database path on the EC2 instance."
  value       = "/var/lib/everpublich/db/everpublich.sqlite"
}

output "btrfs_data_mount_path" {
  description = "Compressed Btrfs data volume mount path on the EC2 instance."
  value       = "/srv/everpublich-data"
}

output "generated_sites_path" {
  description = "Generated static sites root on the EC2 instance."
  value       = "/var/www/everpublich/sites"
}

output "evernote_config_path" {
  description = "Official Evernote client config/cache path when run as the everpublich Linux user."
  value       = "/var/lib/everpublich/.config/Evernote"
}

output "evernote_appimage_path" {
  description = "Installed Evernote AppImage path on the EC2 instance."
  value       = "/opt/everpublich/app/Evernote.AppImage"
}

output "evernote_ssh_x_command" {
  description = "SSH X forwarding command for interactive Evernote login."
  value       = aws_instance.app.public_dns != "" ? "ssh -Y ${var.ssh_user}@${aws_instance.app.public_dns}" : "ssh -Y ${var.ssh_user}@${aws_instance.app.ipv6_addresses[0]}"
}
