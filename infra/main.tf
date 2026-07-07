locals {
  common_tags = {
    Project = var.project_name
  }

  availability_zone                  = var.availability_zone != "" ? var.availability_zone : data.aws_availability_zones.available.names[0]
  base_domain_regex                  = replace(var.base_domain, ".", "\\.")
  cloudfront_origin_id               = "${var.project_name}-s3-origin"
  cloudfront_free_tier_bytes_per_day = floor(var.cloudfront_free_tier_bytes_per_month / 30)
  cloudwatch_namespace               = "Everpublich"
  cloudfront_url                     = var.create_cloudfront_distribution ? "https://${aws_cloudfront_distribution.sites[0].domain_name}/" : ""
  generated_sites_cloudfront_url     = contains(var.cloudfront_aliases, "*.${var.base_domain}") ? "" : local.cloudfront_url
  sites_bucket_name                  = var.sites_bucket_name != "" ? var.sites_bucket_name : "${var.project_name}-${data.aws_caller_identity.current.account_id}-${var.aws_region}-sites"
  ubuntu_image_arch                  = var.instance_architecture == "arm64" ? "arm64" : "amd64"
  zola_target                        = var.instance_architecture == "arm64" ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu"
}

data "aws_caller_identity" "current" {}

data "aws_availability_zones" "available" {
  state = "available"
}

data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"]

  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd-gp3/ubuntu-noble-${var.ubuntu_release}-${local.ubuntu_image_arch}-server-*"]
  }

  filter {
    name   = "architecture"
    values = [var.instance_architecture]
  }

  filter {
    name   = "virtualization-type"
    values = ["hvm"]
  }
}

resource "aws_key_pair" "app" {
  key_name   = var.ssh_key_pair_name
  public_key = file(pathexpand(var.ssh_public_key_path))
  tags       = local.common_tags
}

resource "aws_vpc" "main" {
  assign_generated_ipv6_cidr_block = var.enable_ipv6
  cidr_block                       = var.vpc_cidr
  enable_dns_hostnames             = true
  enable_dns_support               = true

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-vpc"
  })
}

resource "aws_internet_gateway" "main" {
  vpc_id = aws_vpc.main.id

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-internet-gateway"
  })
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.main.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.main.id
  }

  dynamic "route" {
    for_each = var.enable_ipv6 ? [1] : []

    content {
      gateway_id      = aws_internet_gateway.main.id
      ipv6_cidr_block = "::/0"
    }
  }

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-public-routes"
  })
}

resource "aws_subnet" "public" {
  assign_ipv6_address_on_creation = var.enable_ipv6
  availability_zone               = local.availability_zone
  cidr_block                      = var.public_subnet_cidr
  ipv6_cidr_block                 = var.enable_ipv6 ? cidrsubnet(aws_vpc.main.ipv6_cidr_block, 8, 0) : null
  map_public_ip_on_launch         = var.associate_public_ipv4
  vpc_id                          = aws_vpc.main.id

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-public-subnet"
  })
}

resource "aws_route_table_association" "public" {
  route_table_id = aws_route_table.public.id
  subnet_id      = aws_subnet.public.id
}

resource "aws_vpc_endpoint" "s3" {
  route_table_ids   = [aws_route_table.public.id]
  service_name      = "com.amazonaws.${var.aws_region}.s3"
  vpc_endpoint_type = "Gateway"
  vpc_id            = aws_vpc.main.id

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-s3-endpoint"
  })
}

resource "aws_security_group" "app" {
  description = "Everpublich EC2 origin access"
  name        = "${var.project_name}-origin"
  vpc_id      = aws_vpc.main.id

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-origin"
  })
}

resource "aws_vpc_security_group_ingress_rule" "ssh" {
  for_each = toset(var.allowed_ssh_cidrs)

  cidr_ipv4         = each.value
  from_port         = 22
  ip_protocol       = "tcp"
  security_group_id = aws_security_group.app.id
  to_port           = 22
}

resource "aws_vpc_security_group_ingress_rule" "ssh_ipv6" {
  for_each = var.enable_ipv6 ? toset(var.allowed_ssh_ipv6_cidrs) : []

  cidr_ipv6         = each.value
  from_port         = 22
  ip_protocol       = "tcp"
  security_group_id = aws_security_group.app.id
  to_port           = 22
}

resource "aws_vpc_security_group_egress_rule" "all" {
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "-1"
  security_group_id = aws_security_group.app.id
}

resource "aws_vpc_security_group_egress_rule" "all_ipv6" {
  count = var.enable_ipv6 ? 1 : 0

  cidr_ipv6         = "::/0"
  ip_protocol       = "-1"
  security_group_id = aws_security_group.app.id
}

resource "aws_ebs_volume" "data" {
  availability_zone = local.availability_zone
  encrypted         = true
  size              = var.data_volume_size_gb
  type              = "gp3"

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-btrfs-data"
  })
}

resource "aws_s3_bucket" "sites" {
  bucket = local.sites_bucket_name

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-sites"
  })
}

resource "aws_s3_bucket_public_access_block" "sites" {
  block_public_acls       = true
  block_public_policy     = true
  bucket                  = aws_s3_bucket.sites.id
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_ownership_controls" "sites" {
  bucket = aws_s3_bucket.sites.id

  rule {
    object_ownership = "BucketOwnerEnforced"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "sites" {
  bucket = aws_s3_bucket.sites.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_cloudfront_origin_access_control" "sites" {
  count = var.create_cloudfront_distribution ? 1 : 0

  description                       = "Read private Everpublich generated sites from S3"
  name                              = "${var.project_name}-sites"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_cloudfront_function" "host_router" {
  count = var.create_cloudfront_distribution ? 1 : 0

  code    = <<-JS
function handler(event) {
	var request = event.request;
	var hostHeader = request.headers.host;
	var host = hostHeader ? hostHeader.value.toLowerCase() : '';
	var baseDomain = '${var.base_domain}';
	var prefix = '';
	var uri = request.uri;

	if (host.endsWith('.' + baseDomain)) {
		var site = host.substring(0, host.length - baseDomain.length - 1);
		if (site.indexOf('.') !== -1) {
			site = site.substring(0, site.indexOf('.'));
		}
		if (site && site !== 'www') {
			prefix = '/' + site + '/public';
		}
	} else if (uri !== '/') {
		var parts = uri.split('/');
		if (parts.length > 1 && parts[1]) {
			prefix = '/' + parts[1] + '/public';
			uri = '/' + parts.slice(2).join('/');
		}
	}

	var lastSlash = uri.lastIndexOf('/');
	var lastPart = uri.substring(lastSlash + 1);
	if (uri === '/') {
		uri = '/index.html';
	} else if (uri.endsWith('/')) {
		uri += 'index.html';
	} else if (lastPart.indexOf('.') === -1) {
		uri += '/index.html';
	}

	request.uri = prefix + uri;
	return request;
}
JS
  comment = "Route wildcard user subdomains to generated S3 site prefixes."
  name    = "${var.project_name}-host-router"
  publish = true
  runtime = "cloudfront-js-2.0"
}

data "aws_iam_policy_document" "cloudfront_sites_read" {
  count = var.create_cloudfront_distribution ? 1 : 0

  statement {
    actions   = ["s3:GetObject"]
    resources = ["${aws_s3_bucket.sites.arn}/*"]
    sid       = "AllowCloudFrontRead"

    principals {
      identifiers = ["cloudfront.amazonaws.com"]
      type        = "Service"
    }

    condition {
      test     = "StringEquals"
      values   = [aws_cloudfront_distribution.sites[0].arn]
      variable = "AWS:SourceArn"
    }
  }
}

resource "aws_s3_bucket_policy" "sites" {
  count = var.create_cloudfront_distribution ? 1 : 0

  bucket = aws_s3_bucket.sites.id
  policy = data.aws_iam_policy_document.cloudfront_sites_read[0].json
}

resource "aws_iam_role" "app" {
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = "sts:AssumeRole"
        Effect = "Allow"
        Principal = {
          Service = "ec2.amazonaws.com"
        }
      }
    ]
  })
  name = "${var.project_name}-ec2"
  tags = local.common_tags
}

resource "aws_iam_role_policy" "app" {
  name = "${var.project_name}-publish-sites"
  role = aws_iam_role.app.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = concat(
      [
        {
          Action = [
            "s3:ListBucket"
          ]
          Effect   = "Allow"
          Resource = aws_s3_bucket.sites.arn
        },
        {
          Action = [
            "s3:DeleteObject",
            "s3:GetObject",
            "s3:PutObject"
          ]
          Effect   = "Allow"
          Resource = "${aws_s3_bucket.sites.arn}/*"
        }
      ],
      var.create_cloudfront_distribution ? [
        {
          Action = [
            "cloudfront:CreateInvalidation"
          ]
          Effect   = "Allow"
          Resource = aws_cloudfront_distribution.sites[0].arn
        }
      ] : [],
      [
        {
          Action = [
            "cloudwatch:PutMetricData"
          ]
          Condition = {
            StringEquals = {
              "cloudwatch:namespace" = local.cloudwatch_namespace
            }
          }
          Effect   = "Allow"
          Resource = "*"
        }
      ]
    )
  })
}

resource "aws_iam_instance_profile" "app" {
  name = "${var.project_name}-ec2"
  role = aws_iam_role.app.name
}

resource "aws_instance" "app" {
  ami                         = data.aws_ami.ubuntu.id
  associate_public_ip_address = var.associate_public_ipv4
  iam_instance_profile        = aws_iam_instance_profile.app.name
  instance_type               = var.instance_type
  ipv6_address_count          = var.enable_ipv6 ? 1 : 0
  key_name                    = aws_key_pair.app.key_name
  subnet_id                   = aws_subnet.public.id
  user_data_replace_on_change = false
  vpc_security_group_ids      = [aws_security_group.app.id]

  metadata_options {
    http_tokens = "required"
  }

  root_block_device {
    encrypted   = true
    volume_size = var.root_volume_size_gb
    volume_type = "gp3"

    tags = merge(local.common_tags, {
      Name = "${var.project_name}-root"
    })
  }

  user_data_base64 = base64gzip(templatefile("${path.module}/cloud-init.yaml.tftpl", {
    base_domain                = var.base_domain
    base_domain_regex          = local.base_domain_regex
    btrfs_zstd_level           = var.btrfs_zstd_level
    data_volume_id_nodash      = replace(aws_ebs_volume.data.id, "-", "")
    evernote_appimage_regex    = var.evernote_appimage_asset_regex
    evernote_appimage_repo     = var.evernote_appimage_repository
    genius_token               = var.genius_token
    install_evernote_appimage  = var.install_evernote_appimage_on_boot
    install_evernote_gui_deps  = var.install_evernote_gui_deps
    install_rustup_on_boot     = var.install_rustup_on_boot
    install_zola_on_boot       = var.install_zola_on_boot
    project_name               = var.project_name
    run_evernote_on_boot       = var.run_evernote_on_boot
    s3_bucket                  = aws_s3_bucket.sites.bucket
    cloudfront_distribution_id = var.create_cloudfront_distribution ? aws_cloudfront_distribution.sites[0].id : ""
    cloudfront_url             = local.generated_sites_cloudfront_url
    cloudwatch_namespace       = local.cloudwatch_namespace
    sqlite_schema_b64          = base64encode(file("${path.module}/sqlite-schema.sql"))
    install_cloudwatch_agent   = var.install_cloudwatch_agent_on_boot
    support_email              = var.support_email
    support_telegram           = var.support_telegram
    support_tickets            = var.support_tickets
    zola_target                = local.zola_target
    zola_version               = var.zola_version
  }))

  tags = merge(local.common_tags, {
    Name = "${var.project_name}-ec2"
  })

  depends_on = [aws_iam_role_policy.app]
}

resource "aws_volume_attachment" "data" {
  device_name  = "/dev/sdf"
  force_detach = true
  instance_id  = aws_instance.app.id
  volume_id    = aws_ebs_volume.data.id
}

resource "aws_cloudfront_cache_policy" "sites" {
  count = var.create_cloudfront_distribution ? 1 : 0

  comment     = "Cache static Everpublich pages with Brotli and gzip."
  default_ttl = 3600
  max_ttl     = 86400
  min_ttl     = 0
  name        = "${var.project_name}-static-sites"

  parameters_in_cache_key_and_forwarded_to_origin {
    cookies_config {
      cookie_behavior = "none"
    }

    enable_accept_encoding_brotli = true
    enable_accept_encoding_gzip   = true

    headers_config {
      header_behavior = "none"
    }

    query_strings_config {
      query_string_behavior = "all"
    }
  }
}

resource "aws_cloudfront_distribution" "sites" {
  count = var.create_cloudfront_distribution ? 1 : 0

  comment             = "Everpublich generated static websites"
  enabled             = true
  is_ipv6_enabled     = true
  price_class         = var.cloudfront_price_class
  wait_for_deployment = var.cloudfront_wait_for_deployment
  aliases             = var.cloudfront_aliases

  origin {
    domain_name              = aws_s3_bucket.sites.bucket_regional_domain_name
    origin_access_control_id = aws_cloudfront_origin_access_control.sites[0].id
    origin_id                = local.cloudfront_origin_id
  }

  default_cache_behavior {
    allowed_methods        = ["GET", "HEAD", "OPTIONS"]
    cached_methods         = ["GET", "HEAD"]
    cache_policy_id        = aws_cloudfront_cache_policy.sites[0].id
    compress               = true
    target_origin_id       = local.cloudfront_origin_id
    viewer_protocol_policy = "redirect-to-https"

    function_association {
      event_type   = "viewer-request"
      function_arn = aws_cloudfront_function.host_router[0].arn
    }
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn            = length(var.cloudfront_aliases) > 0 ? var.cloudfront_acm_certificate_arn : null
    cloudfront_default_certificate = length(var.cloudfront_aliases) == 0
    minimum_protocol_version       = length(var.cloudfront_aliases) > 0 ? "TLSv1.2_2021" : null
    ssl_support_method             = length(var.cloudfront_aliases) > 0 ? "sni-only" : null
  }

  tags = local.common_tags
}

resource "aws_cloudwatch_dashboard" "operations" {
  count = var.create_cloudwatch_dashboard ? 1 : 0

  dashboard_name = "${var.project_name}-operations"
  dashboard_body = jsonencode({
    widgets = concat(
      [
        {
          height = 6
          width  = 12
          x      = 0
          y      = 0
          type   = "metric"
          properties = {
            title   = "Shared notebooks"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 3600
            stat    = "Maximum"
            metrics = [
              [local.cloudwatch_namespace, "SharedNotebooks", "Service", var.project_name, { label = "Shared notebooks" }]
            ]
          }
        },
        {
          height = 6
          width  = 12
          x      = 12
          y      = 0
          type   = "metric"
          properties = {
            title   = "EC2 CPU and RAM"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 300
            yAxis = {
              left = {
                label = "Percent"
                min   = 0
                max   = 100
              }
            }
            metrics = [
              ["AWS/EC2", "CPUUtilization", "InstanceId", aws_instance.app.id, { id = "cpu", label = "CPU used %", stat = "Average" }],
              [{ expression = "SEARCH('{CWAgent,InstanceId} MetricName=\"mem_used_percent\" InstanceId=\"${aws_instance.app.id}\"', 'Average', 300)", id = "ram", label = "RAM used %" }]
            ]
          }
        },
        {
          height = 6
          width  = 12
          x      = 0
          y      = 6
          type   = "metric"
          properties = {
            title   = "EC2 storage used"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 300
            yAxis = {
              left = {
                label = "Percent"
                min   = 0
                max   = 100
              }
            }
            metrics = [
              [{ expression = "SEARCH('{CWAgent,InstanceId,path} MetricName=\"disk_used_percent\" InstanceId=\"${aws_instance.app.id}\"', 'Average', 300)", id = "disk", label = "Disk used %" }]
            ]
          }
        },
        {
          height = 6
          width  = 12
          x      = 12
          y      = 6
          type   = "metric"
          properties = {
            title   = "S3 storage used"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 86400
            stat    = "Average"
            metrics = [
              ["AWS/S3", "BucketSizeBytes", "BucketName", aws_s3_bucket.sites.bucket, "StorageType", "StandardStorage", { label = "Generated sites bucket bytes" }]
            ]
          }
        },
        {
          height = 6
          width  = 24
          x      = 0
          y      = 18
          type   = "metric"
          properties = {
            title   = "Generation time per website"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 3600
            yAxis = {
              left = {
                label = "Seconds"
                min   = 0
              }
            }
            metrics = [
              [{ expression = "SEARCH('{${local.cloudwatch_namespace},Service,Site} MetricName=\"SiteGenerationSeconds\" Service=\"${var.project_name}\"', 'Minimum', 3600)", id = "gen_min", label = "Min" }],
              [{ expression = "SEARCH('{${local.cloudwatch_namespace},Service,Site} MetricName=\"SiteGenerationSeconds\" Service=\"${var.project_name}\"', 'Average', 3600)", id = "gen_avg", label = "Average" }],
              [{ expression = "SEARCH('{${local.cloudwatch_namespace},Service,Site} MetricName=\"SiteGenerationSeconds\" Service=\"${var.project_name}\"', 'Maximum', 3600)", id = "gen_max", label = "Max" }]
            ]
          }
        },
        {
          height = 6
          width  = 24
          x      = 0
          y      = 24
          type   = "metric"
          properties = {
            title   = "Errors"
            region  = var.aws_region
            view    = "timeSeries"
            stacked = false
            period  = 3600
            metrics = concat(
              [
                [local.cloudwatch_namespace, "BuildFailures", "Service", var.project_name, { label = "Site build failures", stat = "Sum" }],
                [local.cloudwatch_namespace, "SyncErrors", "Service", var.project_name, { label = "Sync command errors", stat = "Sum" }],
                ["AWS/EC2", "StatusCheckFailed", "InstanceId", aws_instance.app.id, { label = "EC2 status check failed", stat = "Maximum" }]
              ],
              var.create_cloudfront_distribution ? [
                ["AWS/CloudFront", "5xxErrorRate", "DistributionId", aws_cloudfront_distribution.sites[0].id, "Region", "Global", { label = "CloudFront 5xx error rate", region = "us-east-1", stat = "Average", yAxis = "right" }],
                ["AWS/CloudFront", "4xxErrorRate", "DistributionId", aws_cloudfront_distribution.sites[0].id, "Region", "Global", { label = "CloudFront 4xx error rate", region = "us-east-1", stat = "Average", yAxis = "right" }]
              ] : []
            )
          }
        }
      ],
      var.create_cloudfront_distribution ? [
        {
          height = 6
          width  = 24
          x      = 0
          y      = 12
          type   = "metric"
          properties = {
            title   = "CloudFront traffic"
            region  = "us-east-1"
            view    = "timeSeries"
            stacked = false
            period  = 86400
            stat    = "Sum"
            yAxis = {
              left = {
                label = "Bytes/day"
                min   = 0
              }
            }
            metrics = [
              ["AWS/CloudFront", "BytesDownloaded", "DistributionId", aws_cloudfront_distribution.sites[0].id, "Region", "Global", { id = "bytes", label = "Bytes downloaded" }],
              [{ expression = "TIME_SERIES(${local.cloudfront_free_tier_bytes_per_day})", id = "free", label = "Free tier daily pace" }]
            ]
          }
        }
      ] : []
    )
  })
}
