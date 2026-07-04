locals {
  common_env = {
    EVERPUBLICH_BASE_DOMAIN        = var.base_domain
    EVERPUBLICH_TOKEN_SECRET       = var.token_secret
    EVERPUBLICH_ADMIN_SECRET       = var.admin_secret
    EVERNOTE_CONSUMER_KEY          = var.evernote_consumer_key
    EVERNOTE_CONSUMER_SECRET       = var.evernote_consumer_secret
    EVERNOTE_SERVICE_ACCOUNT_TOKEN = var.evernote_service_account_token
    GITHUB_OAUTH_CLIENT_ID         = var.github_oauth_client_id
    GITHUB_OAUTH_CLIENT_SECRET     = var.github_oauth_client_secret
    EVERPUBLICH_USERS_TABLE        = aws_dynamodb_table.users.name
    EVERPUBLICH_SITES_BUCKET       = aws_s3_bucket.sites.bucket
    EVERPUBLICH_CLOUDFRONT_ID      = aws_cloudfront_distribution.sites.id
    SUPPORT_EMAIL                  = var.support_email
    SUPPORT_TELEGRAM               = var.support_telegram
    SUPPORT_TICKETS                = var.support_tickets
    RUST_LOG                       = "everpublich=info"
  }
}

resource "aws_dynamodb_table" "users" {
  name         = "${var.project_name}-users"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "pk"
  range_key    = "sk"

  attribute {
    name = "pk"
    type = "S"
  }

  attribute {
    name = "sk"
    type = "S"
  }

  point_in_time_recovery {
    enabled = true
  }
}

resource "aws_s3_bucket" "sites" {
  bucket = "${var.project_name}-sites"
}

resource "aws_s3_bucket_public_access_block" "sites" {
  bucket                  = aws_s3_bucket.sites.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_cloudfront_origin_access_control" "sites" {
  name                              = "${var.project_name}-sites"
  description                       = "Everpublich generated static sites"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_cloudfront_function" "subdomain_prefix" {
  name    = "${var.project_name}-subdomain-prefix"
  runtime = "cloudfront-js-2.0"
  comment = "Map user subdomains to S3 prefixes."
  publish = true
  code    = <<-JS
function handler(event) {
	var request = event.request;
	var host = request.headers.host.value.toLowerCase();
	var base = '${var.base_domain}'.toLowerCase();
	if (host === base || !host.endsWith('.' + base)) {
		return request;
	}
	var subdomain = host.slice(0, 0 - base.length - 1);
	var slash = request.uri.lastIndexOf('/');
	var dot = request.uri.lastIndexOf('.');
	var uri = request.uri;
	if (uri.endsWith('/')) {
		uri = uri + 'index.html';
	} else if (dot < slash) {
		uri = uri + '/index.html';
	}
	if (!uri.startsWith('/' + subdomain + '/')) {
		request.uri = '/' + subdomain + uri;
	} else {
		request.uri = uri;
	}
	return request;
}
JS
}

resource "aws_cloudfront_distribution" "sites" {
  enabled             = true
  default_root_object = "index.html"
  comment             = "${var.project_name} generated sites"
  aliases             = var.acm_certificate_arn == "" ? [] : [var.base_domain, "*.${var.base_domain}"]

  origin {
    domain_name              = aws_s3_bucket.sites.bucket_regional_domain_name
    origin_id                = "sites"
    origin_access_control_id = aws_cloudfront_origin_access_control.sites.id
  }

  default_cache_behavior {
    target_origin_id       = "sites"
    viewer_protocol_policy = "redirect-to-https"
    allowed_methods        = ["GET", "HEAD", "OPTIONS"]
    cached_methods         = ["GET", "HEAD"]
    compress               = true

    forwarded_values {
      query_string = false

      cookies {
        forward = "none"
      }
    }

    function_association {
      event_type   = "viewer-request"
      function_arn = aws_cloudfront_function.subdomain_prefix.arn
    }
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn            = var.acm_certificate_arn == "" ? null : var.acm_certificate_arn
    cloudfront_default_certificate = var.acm_certificate_arn == ""
    minimum_protocol_version       = var.acm_certificate_arn == "" ? null : "TLSv1.2_2021"
    ssl_support_method             = var.acm_certificate_arn == "" ? null : "sni-only"
  }
}

resource "aws_s3_bucket_policy" "sites" {
  bucket = aws_s3_bucket.sites.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "AllowCloudFrontRead"
        Effect = "Allow"
        Principal = {
          Service = "cloudfront.amazonaws.com"
        }
        Action   = "s3:GetObject"
        Resource = "${aws_s3_bucket.sites.arn}/*"
        Condition = {
          StringEquals = {
            "AWS:SourceArn" = aws_cloudfront_distribution.sites.arn
          }
        }
      }
    ]
  })
}

resource "aws_iam_role" "lambda" {
  name = "${var.project_name}-lambda"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = "sts:AssumeRole"
        Effect = "Allow"
        Principal = {
          Service = "lambda.amazonaws.com"
        }
      }
    ]
  })
}

resource "aws_iam_role_policy_attachment" "basic_execution" {
  role       = aws_iam_role.lambda.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_iam_role_policy" "app" {
  name = "${var.project_name}-app"
  role = aws_iam_role.lambda.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "dynamodb:GetItem",
          "dynamodb:PutItem",
          "dynamodb:UpdateItem",
          "dynamodb:DeleteItem",
          "dynamodb:Query",
          "dynamodb:Scan"
        ]
        Resource = aws_dynamodb_table.users.arn
      },
      {
        Effect = "Allow"
        Action = [
          "s3:GetObject",
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:ListBucket"
        ]
        Resource = [
          aws_s3_bucket.sites.arn,
          "${aws_s3_bucket.sites.arn}/*"
        ]
      },
      {
        Effect   = "Allow"
        Action   = ["cloudfront:CreateInvalidation"]
        Resource = aws_cloudfront_distribution.sites.arn
      }
    ]
  })
}

resource "aws_cloudwatch_log_group" "api" {
  name              = "/aws/lambda/${var.project_name}-api"
  retention_in_days = 14
}

resource "aws_lambda_function" "api" {
  function_name = "${var.project_name}-api"
  role          = aws_iam_role.lambda.arn
  filename      = var.api_lambda_zip_path

  source_code_hash = filebase64sha256(var.api_lambda_zip_path)
  runtime          = "provided.al2023"
  handler          = "bootstrap"
  architectures    = ["arm64"]
  memory_size      = var.lambda_memory_size_mb
  timeout          = 30

  environment {
    variables = local.common_env
  }

  depends_on = [
    aws_cloudwatch_log_group.api,
    aws_iam_role_policy_attachment.basic_execution,
    aws_iam_role_policy.app
  ]
}

resource "aws_lambda_function_url" "api" {
  function_name      = aws_lambda_function.api.function_name
  authorization_type = "NONE"

  cors {
    allow_headers = ["authorization", "content-type"]
    allow_methods = ["GET", "POST"]
    allow_origins = ["*"]
    max_age       = 86400
  }
}

resource "aws_cloudwatch_log_group" "worker" {
  for_each          = var.builder_users
  name              = "/aws/lambda/${var.project_name}-builder-${each.key}"
  retention_in_days = 14
}

resource "aws_lambda_function" "worker" {
  for_each      = var.builder_users
  function_name = "${var.project_name}-builder-${each.key}"
  role          = aws_iam_role.lambda.arn
  filename      = var.worker_lambda_zip_path

  source_code_hash = filebase64sha256(var.worker_lambda_zip_path)
  runtime          = "provided.al2023"
  handler          = "bootstrap"
  architectures    = ["arm64"]
  memory_size      = var.lambda_memory_size_mb
  timeout          = var.lambda_timeout_seconds

  ephemeral_storage {
    size = var.lambda_ephemeral_storage_mb
  }

  environment {
    variables = merge(local.common_env, {
      EVERPUBLICH_USER_ID    = each.key
      EVERPUBLICH_SUBDOMAIN  = each.value.subdomain
      EVERPUBLICH_BUILD_MODE = "full_regeneration"
    })
  }

  depends_on = [
    aws_cloudwatch_log_group.worker,
    aws_iam_role_policy_attachment.basic_execution,
    aws_iam_role_policy.app
  ]
}

resource "aws_cloudwatch_event_rule" "daily_worker" {
  for_each            = var.builder_users
  name                = "${var.project_name}-daily-${each.key}"
  description         = "Daily full regeneration for Everpublich user ${each.key}"
  schedule_expression = "rate(1 day)"
}

resource "aws_cloudwatch_event_target" "daily_worker" {
  for_each = var.builder_users
  rule     = aws_cloudwatch_event_rule.daily_worker[each.key].name
  arn      = aws_lambda_function.worker[each.key].arn
}

resource "aws_lambda_permission" "allow_eventbridge" {
  for_each      = var.builder_users
  statement_id  = "AllowExecutionFromEventBridge"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.worker[each.key].function_name
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.daily_worker[each.key].arn
}

resource "aws_route53_record" "wildcard" {
  count   = var.route53_zone_id == "" ? 0 : 1
  zone_id = var.route53_zone_id
  name    = "*.${var.base_domain}"
  type    = "CNAME"
  ttl     = 300
  records = [aws_cloudfront_distribution.sites.domain_name]
}
