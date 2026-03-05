<?php
namespace Magento\Framework\App;

class PromoClass
{
    public function __construct(
        public readonly \Magento\Framework\App\RequestInterface $request,
        protected \Magento\Framework\App\ResponseInterface $response,
        private string $name = 'default',
        public int $priority = 0
    ) {}
}
