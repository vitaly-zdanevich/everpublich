output "api_function_url" {
  description = "Public Lambda Function URL for landing, OAuth, and admin routes."
  value       = aws_lambda_function_url.api.function_url
}

output "users_table_name" {
  description = "DynamoDB users table."
  value       = aws_dynamodb_table.users.name
}

output "sites_bucket_name" {
  description = "S3 bucket for generated static sites and media."
  value       = aws_s3_bucket.sites.bucket
}

output "cloudfront_domain_name" {
  description = "CloudFront distribution domain."
  value       = aws_cloudfront_distribution.sites.domain_name
}
