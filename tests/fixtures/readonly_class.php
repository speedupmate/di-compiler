<?php
namespace Magento\Framework;

readonly class ValueObject
{
    public function __construct(
        public readonly string $id,
        public readonly string $name
    ) {}
}
